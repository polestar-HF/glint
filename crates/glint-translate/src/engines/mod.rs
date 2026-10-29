//! The engines, and the glue shared by the non-LLM ones.

pub(crate) mod anthropic;
pub(crate) mod baidu;
pub(crate) mod bing;
pub(crate) mod deepl;
pub(crate) mod google;
pub(crate) mod openai;
pub(crate) mod youdao;

use std::sync::atomic::{AtomicU64, Ordering};
use std::time::{SystemTime, UNIX_EPOCH};

use crate::http::{retrying, within, Failure, MT_TIMEOUT};
use crate::langs::{self, Role};
use crate::{DeltaSink, EngineConfig, Result, TranslateError, TranslateRequest, Translator};

pub(crate) const DEFAULT_ANTHROPIC_MODEL: &str = "claude-haiku-5-5";
pub(crate) const DEFAULT_OPENAI_MODEL: &str = "gpt-4.1-mini";

pub(crate) fn default_name(kind: &str) -> &'static str {
    match kind {
        "anthropic" => "Claude",
        "openai" => "OpenAI",
        "google" => "Google",
        "bing" => "Bing",
        "deepl" => "DeepL",
        "youdao" => "Youdao",
        "baidu" => "Baidu",
        _ => "Translator",
    }
}

/// The model an engine will really use, with the per-kind default filled in. Empty for
/// engines that have no model.
pub(crate) fn effective_model(cfg: &EngineConfig) -> String {
    let set = non_empty(cfg.model.as_deref());
    match cfg.kind.as_str() {
        "anthropic" => set.unwrap_or(DEFAULT_ANTHROPIC_MODEL).to_string(),
        "openai" => set.unwrap_or(DEFAULT_OPENAI_MODEL).to_string(),
        _ => set.unwrap_or("").to_string(),
    }
}

/// A trimmed value, or `None` if it is absent or blank (settings forms often save `""`).
pub(crate) fn non_empty(v: Option<&str>) -> Option<&str> {
    v.map(str::trim).filter(|s| !s.is_empty())
}

pub(crate) fn build(cfg: &EngineConfig, http: &reqwest::Client) -> Result<Box<dyn Translator>> {
    let id = cfg.id.clone();
    let name = non_empty(cfg.name.as_deref())
        .map(str::to_owned)
        .unwrap_or_else(|| default_name(&cfg.kind).to_string());
    let http = http.clone();
    Ok(match cfg.kind.as_str() {
        "anthropic" => Box::new(anthropic::Anthropic::new(id, name, cfg, http)?),
        "openai" => Box::new(openai::OpenAi::new(id, name, cfg, http)?),
        "google" => Box::new(MtEngine::new(id, name, google::Google::new(cfg, http))),
        "bing" => Box::new(MtEngine::new(id, name, bing::Bing::new(cfg, http))),
        "deepl" => Box::new(MtEngine::new(id, name, deepl::DeepL::new(cfg, http)?)),
        "youdao" => Box::new(MtEngine::new(id, name, youdao::Youdao::new(cfg, http)?)),
        "baidu" => Box::new(MtEngine::new(id, name, baidu::Baidu::new(cfg, http)?)),
        other => {
            return Err(TranslateError::Config(format!(
                "Unknown translation engine \"{other}\""
            )))
        }
    })
}

/// What one machine-translation call returns.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct MtOut {
    pub text: String,
    /// The language the engine says it detected, in the engine's own spelling.
    pub detected: Option<String>,
}

/// One machine-translation service. [`MtEngine`] adds everything the engines have in common:
/// language resolution, the retry, flipping to the secondary language, and the single delta.
#[async_trait::async_trait]
pub(crate) trait Backend: Send + Sync {
    /// Map a BCP-47 code to the engine's code.
    fn lang(&self, code: &str, role: Role) -> Result<&'static str>;

    /// Translate `text` once. `from` is the engine's code for the source, which can mean
    /// "detect" (an empty string or `auto`, depending on the engine).
    async fn call(&self, text: &str, from: &str, to: &str) -> std::result::Result<MtOut, Failure>;

    /// Refuse texts the service cannot take, with a clear message instead of a vague error.
    fn check_len(&self, _text: &str) -> Result<()> {
        Ok(())
    }
}

pub(crate) struct MtEngine<B: Backend> {
    id: String,
    name: String,
    backend: B,
}

impl<B: Backend> MtEngine<B> {
    fn new(id: String, name: String, backend: B) -> Self {
        Self { id, name, backend }
    }
}

#[async_trait::async_trait]
impl<B: Backend> Translator for MtEngine<B> {
    fn id(&self) -> &str {
        &self.id
    }

    fn name(&self) -> &str {
        &self.name
    }

    async fn translate(&self, req: &TranslateRequest, deltas: DeltaSink) -> Result<String> {
        let text = req.text.trim();
        if text.is_empty() {
            return Ok(String::new());
        }
        self.backend.check_len(text)?;

        let secondary = req.secondary_lang.as_deref();
        let target = langs::effective_target(&req.source_lang, &req.target_lang, secondary);
        if !langs::is_auto(&req.source_lang) && langs::same_lang(&req.source_lang, &target) {
            // Same language and no secondary to switch to: there is nothing to do.
            let _ = deltas.send(text.to_string());
            return Ok(text.to_string());
        }
        let from = self.backend.lang(&req.source_lang, Role::Source)?;
        let to = self.backend.lang(&target, Role::Target)?;

        let out = within(MT_TIMEOUT, async {
            let mut out = retrying(|| self.backend.call(text, from, to)).await?;

            // Source "auto" and the text turned out to be in the target language already:
            // translate it into the secondary language instead (one more request, only in
            // this case).
            if langs::is_auto(&req.source_lang) {
                if let (Some(detected), Some(sec)) = (
                    out.detected.as_deref(),
                    langs::usable_secondary(&target, secondary),
                ) {
                    if langs::same_engine_lang(detected, to) {
                        let to2 = self.backend.lang(sec, Role::Target)?;
                        out = retrying(|| self.backend.call(text, from, to2)).await?;
                    }
                }
            }
            Ok(out)
        })
        .await?;

        let _ = deltas.send(out.text.clone());
        Ok(out.text)
    }
}

/// A salt that is unique per request: the clock in nanoseconds plus a process-wide counter,
/// as decimal digits (Baidu wants a number, Youdao accepts any string).
pub(crate) fn unique_salt() -> String {
    static COUNTER: AtomicU64 = AtomicU64::new(0);
    let nanos = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_nanos())
        .unwrap_or(0);
    let n = COUNTER.fetch_add(1, Ordering::Relaxed);
    format!("{}{:04}", nanos % 1_000_000_000_000, n % 10_000)
}

/// Lower-case hex of `bytes`.
pub(crate) fn hex(bytes: &[u8]) -> String {
    const DIGITS: &[u8; 16] = b"0123456789abcdef";
    let mut s = String::with_capacity(bytes.len() * 2);
    for b in bytes {
        s.push(DIGITS[(b >> 4) as usize] as char);
        s.push(DIGITS[(b & 0x0f) as usize] as char);
    }
    s
}

/// A `Parse` error that never includes the response body.
pub(crate) fn parse_err(what: &str) -> TranslateError {
    TranslateError::Parse(what.to_string())
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::AtomicUsize;
    use std::sync::{Arc, Mutex};

    #[test]
    fn hex_is_lowercase_and_padded() {
        assert_eq!(hex(&[0x00, 0x0f, 0xa0, 0xff]), "000fa0ff");
    }

    #[test]
    fn salts_are_unique_digits() {
        let a = unique_salt();
        let b = unique_salt();
        assert_ne!(a, b);
        assert!(a.chars().all(|c| c.is_ascii_digit()));
    }

    #[test]
    fn effective_model_fills_defaults_and_trims() {
        let mut c = crate::default_engines().remove(2);
        c.model = None;
        assert_eq!(effective_model(&c), "claude-haiku-5-5");
        c.model = Some("  ".into());
        assert_eq!(effective_model(&c), "claude-haiku-5-5");
        c.model = Some(" claude-sonnet-5-5 ".into());
        assert_eq!(effective_model(&c), "claude-sonnet-5-5");
        c.kind = "google".into();
        c.model = None;
        assert_eq!(effective_model(&c), "");
    }

    type Calls = Arc<Mutex<Vec<(String, String)>>>;

    /// A scripted backend: answers per call with (text, detected), records (from, to).
    struct Script {
        answers: Mutex<Vec<MtOut>>,
        calls: Calls,
        count: AtomicUsize,
    }

    impl Script {
        fn new(answers: Vec<MtOut>) -> (Self, Calls) {
            let calls = Arc::new(Mutex::new(Vec::new()));
            (
                Script {
                    answers: Mutex::new(answers),
                    calls: calls.clone(),
                    count: AtomicUsize::new(0),
                },
                calls,
            )
        }
    }

    #[async_trait::async_trait]
    impl Backend for Script {
        fn lang(&self, code: &str, role: Role) -> Result<&'static str> {
            langs::google(code, role)
        }
        async fn call(
            &self,
            _text: &str,
            from: &str,
            to: &str,
        ) -> std::result::Result<MtOut, Failure> {
            self.count.fetch_add(1, Ordering::SeqCst);
            self.calls
                .lock()
                .unwrap_or_else(|p| p.into_inner())
                .push((from.into(), to.into()));
            let mut a = self.answers.lock().unwrap_or_else(|p| p.into_inner());
            Ok(a.remove(0))
        }
    }

    fn out(text: &str, detected: Option<&str>) -> MtOut {
        MtOut {
            text: text.into(),
            detected: detected.map(str::to_owned),
        }
    }

    async fn run(
        engine: &impl Translator,
        req: &TranslateRequest,
    ) -> (Result<String>, Vec<String>) {
        let (tx, mut rx) = tokio::sync::mpsc::unbounded_channel();
        let res = engine.translate(req, tx).await;
        let mut deltas = Vec::new();
        while let Ok(d) = rx.try_recv() {
            deltas.push(d);
        }
        (res, deltas)
    }

    #[tokio::test]
    async fn plain_translation_sends_one_delta() {
        let (b, calls) = Script::new(vec![out("你好", Some("en"))]);
        let e = MtEngine::new("g".into(), "G".into(), b);
        let mut req = TranslateRequest::new("hello", "auto", "zh-Hans");
        req.secondary_lang = Some("en".into());
        let (res, deltas) = run(&e, &req).await;
        assert_eq!(res.ok().as_deref(), Some("你好"));
        assert_eq!(deltas, vec!["你好".to_string()]);
        assert_eq!(
            *calls.lock().unwrap_or_else(|p| p.into_inner()),
            vec![("auto".to_string(), "zh-CN".to_string())]
        );
    }

    #[tokio::test]
    async fn auto_source_in_target_language_flips_to_secondary() {
        let (b, calls) = Script::new(vec![
            out("你好", Some("zh-CN")),
            out("Hello", Some("zh-CN")),
        ]);
        let e = MtEngine::new("g".into(), "G".into(), b);
        let mut req = TranslateRequest::new("你好", "auto", "zh-Hans");
        req.secondary_lang = Some("en".into());
        let (res, deltas) = run(&e, &req).await;
        assert_eq!(res.ok().as_deref(), Some("Hello"));
        assert_eq!(
            deltas,
            vec!["Hello".to_string()],
            "only the final text is shown"
        );
        let calls = calls.lock().unwrap_or_else(|p| p.into_inner()).clone();
        assert_eq!(
            calls,
            vec![
                ("auto".into(), "zh-CN".into()),
                ("auto".into(), "en".into())
            ]
        );
    }

    #[tokio::test]
    async fn no_flip_without_a_usable_secondary() {
        for sec in [None, Some("zh-Hans"), Some("auto")] {
            let (b, calls) = Script::new(vec![out("你好", Some("zh-CN"))]);
            let e = MtEngine::new("g".into(), "G".into(), b);
            let mut req = TranslateRequest::new("你好", "auto", "zh-Hans");
            req.secondary_lang = sec.map(str::to_owned);
            let (res, _) = run(&e, &req).await;
            assert_eq!(res.ok().as_deref(), Some("你好"), "{sec:?}");
            assert_eq!(
                calls.lock().unwrap_or_else(|p| p.into_inner()).len(),
                1,
                "{sec:?}"
            );
        }
    }

    #[tokio::test]
    async fn chinese_scripts_are_not_confused_when_flipping() {
        // Traditional text, simplified target: that is a real translation, not "already in target".
        let (b, calls) = Script::new(vec![out("你好", Some("zh-TW"))]);
        let e = MtEngine::new("g".into(), "G".into(), b);
        let mut req = TranslateRequest::new("你好", "auto", "zh-Hans");
        req.secondary_lang = Some("en".into());
        let (res, _) = run(&e, &req).await;
        assert!(res.is_ok());
        assert_eq!(calls.lock().unwrap_or_else(|p| p.into_inner()).len(), 1);
    }

    #[tokio::test]
    async fn explicit_source_equal_to_target_uses_secondary_or_returns_the_text() {
        let (b, calls) = Script::new(vec![out("你好", None)]);
        let e = MtEngine::new("g".into(), "G".into(), b);
        let mut req = TranslateRequest::new("hello", "en", "en");
        req.secondary_lang = Some("zh-Hans".into());
        let (res, _) = run(&e, &req).await;
        assert_eq!(res.ok().as_deref(), Some("你好"));
        assert_eq!(
            calls.lock().unwrap_or_else(|p| p.into_inner())[0],
            ("en".to_string(), "zh-CN".to_string())
        );

        let (b, calls) = Script::new(vec![]);
        let e = MtEngine::new("g".into(), "G".into(), b);
        let req = TranslateRequest::new("hello", "en", "en");
        let (res, deltas) = run(&e, &req).await;
        assert_eq!(res.ok().as_deref(), Some("hello"));
        assert_eq!(deltas, vec!["hello".to_string()]);
        assert!(
            calls.lock().unwrap_or_else(|p| p.into_inner()).is_empty(),
            "no request is made"
        );
    }

    #[tokio::test]
    async fn blank_text_makes_no_request_and_unsupported_languages_fail_clearly() {
        let (b, calls) = Script::new(vec![]);
        let e = MtEngine::new("g".into(), "G".into(), b);
        let (res, deltas) = run(&e, &TranslateRequest::new("  \n ", "auto", "en")).await;
        assert_eq!(res.ok().as_deref(), Some(""));
        assert!(deltas.is_empty());
        assert!(calls.lock().unwrap_or_else(|p| p.into_inner()).is_empty());

        let (res, _) = run(&e, &TranslateRequest::new("hi", "auto", "tlh")).await;
        assert!(matches!(res, Err(TranslateError::Config(m)) if m.contains("tlh")));
        let (res, _) = run(&e, &TranslateRequest::new("hi", "auto", "auto")).await;
        assert!(matches!(res, Err(TranslateError::Config(_))));
    }

    #[test]
    fn default_names() {
        assert_eq!(default_name("anthropic"), "Claude");
        assert_eq!(default_name("deepl"), "DeepL");
        assert_eq!(default_name("weird"), "Translator");
    }
}
