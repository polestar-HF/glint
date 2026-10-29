//! In-memory LRU cache of finished translations.
//!
//! One [`Cache`] is shared by the whole app (it is a cheap `Clone` around an `Arc`). Engines are
//! wrapped with [`Cache::wrap`] (or built with [`crate::build_cached`]); a hit replays the text
//! as a single delta, so the UI behaves as it does for a non-streaming engine.
//!
//! The key is (engine id, model, source, target, digest), where the digest covers the text and
//! everything else that changes what an engine returns: the secondary language, the user's extra
//! prompt, and the base URL. Failures are never cached.

use std::collections::HashMap;
use std::sync::{Arc, Mutex, MutexGuard};

use sha2::{Digest, Sha256};

use crate::{DeltaSink, EngineConfig, Result, TranslateRequest, Translator};

/// Default capacity, in entries.
pub const DEFAULT_CAPACITY: usize = 200;

#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct CacheKey {
    engine: String,
    model: String,
    source: String,
    target: String,
    digest: [u8; 16],
}

impl CacheKey {
    /// `variant` carries engine settings that affect the output (prompt, base URL).
    pub fn new(engine_id: &str, model: &str, variant: &str, req: &TranslateRequest) -> Self {
        let mut h = Sha256::new();
        // Length-prefix every field so ("ab","c") and ("a","bc") cannot collide.
        for part in [
            variant,
            req.secondary_lang.as_deref().unwrap_or(""),
            req.text.as_str(),
        ] {
            h.update((part.len() as u64).to_le_bytes());
            h.update(part.as_bytes());
        }
        let full = h.finalize();
        let mut digest = [0u8; 16];
        digest.copy_from_slice(&full[..16]);
        CacheKey {
            engine: engine_id.to_string(),
            model: model.to_string(),
            source: req.source_lang.trim().to_ascii_lowercase(),
            target: req.target_lang.trim().to_ascii_lowercase(),
            digest,
        }
    }
}

#[derive(Debug)]
struct Inner {
    capacity: usize,
    tick: u64,
    map: HashMap<CacheKey, (String, u64)>,
}

/// A shared LRU cache. Clones share the same storage.
#[derive(Debug, Clone)]
pub struct Cache {
    inner: Arc<Mutex<Inner>>,
}

impl Default for Cache {
    fn default() -> Self {
        Cache::new(DEFAULT_CAPACITY)
    }
}

impl Cache {
    pub fn new(capacity: usize) -> Self {
        Cache {
            inner: Arc::new(Mutex::new(Inner {
                capacity: capacity.max(1),
                tick: 0,
                map: HashMap::new(),
            })),
        }
    }

    fn lock(&self) -> MutexGuard<'_, Inner> {
        // A panic while holding the lock cannot leave the map half-updated in a way that
        // matters for a cache, so keep going with whatever is there.
        self.inner.lock().unwrap_or_else(|p| p.into_inner())
    }

    pub fn get(&self, key: &CacheKey) -> Option<String> {
        let mut g = self.lock();
        g.tick += 1;
        let tick = g.tick;
        g.map.get_mut(key).map(|(text, used)| {
            *used = tick;
            text.clone()
        })
    }

    pub fn insert(&self, key: CacheKey, text: String) {
        let mut g = self.lock();
        g.tick += 1;
        let tick = g.tick;
        g.map.insert(key, (text, tick));
        while g.map.len() > g.capacity {
            // At most a few hundred entries, so a scan for the oldest is cheaper than
            // maintaining a linked list.
            let oldest = g
                .map
                .iter()
                .min_by_key(|(_, (_, used))| *used)
                .map(|(k, _)| k.clone());
            match oldest {
                Some(k) => {
                    g.map.remove(&k);
                }
                None => break,
            }
        }
    }

    pub fn len(&self) -> usize {
        self.lock().map.len()
    }

    pub fn is_empty(&self) -> bool {
        self.len() == 0
    }

    pub fn clear(&self) {
        self.lock().map.clear();
    }

    /// Wrap `inner` so its results go through this cache. `cfg` supplies the key parts that
    /// are fixed per engine.
    pub fn wrap(&self, inner: Box<dyn Translator>, cfg: &EngineConfig) -> Box<dyn Translator> {
        Box::new(CachedTranslator {
            inner,
            cache: self.clone(),
            engine_id: cfg.id.clone(),
            model: crate::engines::effective_model(cfg),
            variant: format!(
                "{}\u{0}{}\u{0}{}",
                cfg.kind,
                cfg.base_url.as_deref().map(str::trim).unwrap_or(""),
                cfg.prompt.as_deref().map(str::trim).unwrap_or("")
            ),
        })
    }
}

struct CachedTranslator {
    inner: Box<dyn Translator>,
    cache: Cache,
    engine_id: String,
    model: String,
    variant: String,
}

#[async_trait::async_trait]
impl Translator for CachedTranslator {
    fn id(&self) -> &str {
        self.inner.id()
    }

    fn name(&self) -> &str {
        self.inner.name()
    }

    async fn translate(&self, req: &TranslateRequest, deltas: DeltaSink) -> Result<String> {
        let key = CacheKey::new(&self.engine_id, &self.model, &self.variant, req);
        if let Some(hit) = self.cache.get(&key) {
            let _ = deltas.send(hit.clone());
            return Ok(hit);
        }
        let text = self.inner.translate(req, deltas).await?;
        if !text.trim().is_empty() {
            self.cache.insert(key, text.clone());
        }
        Ok(text)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::{AtomicUsize, Ordering};

    fn req(text: &str) -> TranslateRequest {
        TranslateRequest::new(text, "auto", "zh-Hans")
    }

    fn key(engine: &str, text: &str) -> CacheKey {
        CacheKey::new(engine, "m", "v", &req(text))
    }

    #[test]
    fn hit_miss_and_clear() {
        let c = Cache::new(4);
        assert!(c.is_empty());
        assert_eq!(c.get(&key("e", "a")), None);
        c.insert(key("e", "a"), "A".into());
        assert_eq!(c.get(&key("e", "a")).as_deref(), Some("A"));
        assert_eq!(c.get(&key("other", "a")), None);
        assert_eq!(c.len(), 1);
        c.clear();
        assert!(c.is_empty());
    }

    #[test]
    fn evicts_the_least_recently_used_entry() {
        let c = Cache::new(3);
        c.insert(key("e", "1"), "1".into());
        c.insert(key("e", "2"), "2".into());
        c.insert(key("e", "3"), "3".into());
        // Touch 1, so 2 is now the oldest.
        assert!(c.get(&key("e", "1")).is_some());
        c.insert(key("e", "4"), "4".into());
        assert_eq!(c.len(), 3);
        assert!(
            c.get(&key("e", "2")).is_none(),
            "2 should have been evicted"
        );
        for t in ["1", "3", "4"] {
            assert!(c.get(&key("e", t)).is_some(), "{t} should remain");
        }
    }

    #[test]
    fn default_capacity_is_200() {
        let c = Cache::default();
        for i in 0..250 {
            c.insert(key("e", &i.to_string()), "x".into());
        }
        assert_eq!(c.len(), 200);
        assert!(c.get(&key("e", "0")).is_none());
        assert!(c.get(&key("e", "249")).is_some());
    }

    #[test]
    fn key_distinguishes_every_input() {
        let base = CacheKey::new("e", "m", "v", &req("hello"));
        assert_eq!(base, CacheKey::new("e", "m", "v", &req("hello")));
        assert_ne!(base, CacheKey::new("e2", "m", "v", &req("hello")));
        assert_ne!(base, CacheKey::new("e", "m2", "v", &req("hello")));
        assert_ne!(base, CacheKey::new("e", "m", "v2", &req("hello")));
        assert_ne!(base, CacheKey::new("e", "m", "v", &req("hello!")));
        assert_ne!(
            base,
            CacheKey::new(
                "e",
                "m",
                "v",
                &TranslateRequest::new("hello", "en", "zh-Hans")
            )
        );
        assert_ne!(
            base,
            CacheKey::new("e", "m", "v", &TranslateRequest::new("hello", "auto", "ja"))
        );
        let mut with_secondary = req("hello");
        with_secondary.secondary_lang = Some("en".into());
        assert_ne!(base, CacheKey::new("e", "m", "v", &with_secondary));
        // Language codes are compared case-insensitively.
        assert_eq!(
            CacheKey::new(
                "e",
                "m",
                "v",
                &TranslateRequest::new("hello", "AUTO", "ZH-HANS")
            ),
            base
        );
    }

    #[test]
    fn digest_fields_cannot_run_together() {
        let mut a = req("bc");
        a.secondary_lang = Some("a".into());
        let mut b = req("c");
        b.secondary_lang = Some("ab".into());
        assert_ne!(
            CacheKey::new("e", "m", "v", &a),
            CacheKey::new("e", "m", "v", &b)
        );
    }

    struct Counting {
        calls: Arc<AtomicUsize>,
        fail: bool,
    }

    #[async_trait::async_trait]
    impl Translator for Counting {
        fn id(&self) -> &str {
            "counting"
        }
        fn name(&self) -> &str {
            "Counting"
        }
        async fn translate(&self, req: &TranslateRequest, deltas: DeltaSink) -> Result<String> {
            self.calls.fetch_add(1, Ordering::SeqCst);
            if self.fail {
                return Err(crate::TranslateError::Network("down".into()));
            }
            let out = format!("T:{}", req.text);
            let _ = deltas.send(out.clone());
            Ok(out)
        }
    }

    fn cfg() -> EngineConfig {
        let mut c = crate::default_engines().remove(0);
        c.id = "counting".into();
        c
    }

    #[tokio::test]
    async fn wrapper_serves_repeats_from_cache_and_replays_one_delta() {
        let calls = Arc::new(AtomicUsize::new(0));
        let cache = Cache::default();
        let t = cache.wrap(
            Box::new(Counting {
                calls: calls.clone(),
                fail: false,
            }),
            &cfg(),
        );

        let (tx, mut rx) = tokio::sync::mpsc::unbounded_channel();
        assert_eq!(
            t.translate(&req("hi"), tx).await.ok().as_deref(),
            Some("T:hi")
        );
        assert_eq!(rx.recv().await.as_deref(), Some("T:hi"));

        let (tx, mut rx) = tokio::sync::mpsc::unbounded_channel();
        assert_eq!(
            t.translate(&req("hi"), tx).await.ok().as_deref(),
            Some("T:hi")
        );
        assert_eq!(rx.recv().await.as_deref(), Some("T:hi"));
        assert!(rx.recv().await.is_none(), "a hit sends exactly one delta");
        assert_eq!(calls.load(Ordering::SeqCst), 1);

        // A different text misses.
        let (tx, _rx) = tokio::sync::mpsc::unbounded_channel();
        assert!(t.translate(&req("other"), tx).await.is_ok());
        assert_eq!(calls.load(Ordering::SeqCst), 2);
        assert_eq!(t.id(), "counting");
        assert_eq!(t.name(), "Counting");
    }

    #[tokio::test]
    async fn clones_share_storage_and_failures_are_not_cached() {
        let calls = Arc::new(AtomicUsize::new(0));
        let cache = Cache::default();
        let failing = cache.clone().wrap(
            Box::new(Counting {
                calls: calls.clone(),
                fail: true,
            }),
            &cfg(),
        );
        let (tx, _rx) = tokio::sync::mpsc::unbounded_channel();
        assert!(failing.translate(&req("hi"), tx).await.is_err());
        assert!(cache.is_empty());

        let ok = cache.wrap(
            Box::new(Counting {
                calls: calls.clone(),
                fail: false,
            }),
            &cfg(),
        );
        let (tx, _rx) = tokio::sync::mpsc::unbounded_channel();
        assert!(ok.translate(&req("hi"), tx).await.is_ok());
        assert_eq!(cache.len(), 1);
    }
}
