//! Lazily loaded OCR engine with idle unloading (ONNX sessions cost 100+ MB while resident).

use std::path::PathBuf;
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use glint_ocr::OcrEngine;

struct Loaded {
    kind: String,
    engine: Arc<dyn OcrEngine>,
    last_used: Instant,
}

pub struct OcrService {
    model_dir: PathBuf,
    slot: Mutex<Option<Loaded>>,
    /// Serializes loads so a preload and a real request never load twice.
    load_lock: Mutex<()>,
}

impl OcrService {
    pub fn new(model_dir: PathBuf) -> Self {
        Self { model_dir, slot: Mutex::new(None), load_lock: Mutex::new(()) }
    }

    /// Return the engine for `kind`, loading it if needed. Blocking; call off the UI thread.
    pub fn get(&self, kind: &str) -> anyhow::Result<Arc<dyn OcrEngine>> {
        if let Some(engine) = self.cached(kind) {
            return Ok(engine);
        }
        let _guard = self.load_lock.lock().unwrap_or_else(|p| p.into_inner());
        if let Some(engine) = self.cached(kind) {
            return Ok(engine);
        }
        let started = Instant::now();
        let engine = self.load(kind)?;
        log::info!("ocr engine '{kind}' loaded in {:?}", started.elapsed());
        *self.slot.lock().unwrap_or_else(|p| p.into_inner()) =
            Some(Loaded { kind: kind.to_string(), engine: engine.clone(), last_used: Instant::now() });
        Ok(engine)
    }

    /// Warm the engine in the background (called when the capture hotkey fires, so model
    /// loading overlaps the user's drag).
    pub fn preload(self: &Arc<Self>, kind: String) {
        let this = self.clone();
        std::thread::spawn(move || {
            if let Err(e) = this.get(&kind) {
                log::warn!("ocr preload failed: {e:#}");
            }
        });
    }

    /// Drop the engine if unused for `idle`. Called periodically.
    pub fn unload_if_idle(&self, idle: Duration) {
        let mut slot = self.slot.lock().unwrap_or_else(|p| p.into_inner());
        if slot.as_ref().is_some_and(|l| l.last_used.elapsed() >= idle) {
            log::info!("unloading idle ocr engine");
            *slot = None;
        }
    }

    fn cached(&self, kind: &str) -> Option<Arc<dyn OcrEngine>> {
        let mut slot = self.slot.lock().unwrap_or_else(|p| p.into_inner());
        match slot.as_mut() {
            Some(l) if l.kind == kind => {
                l.last_used = Instant::now();
                Some(l.engine.clone())
            }
            _ => None,
        }
    }

    fn load(&self, kind: &str) -> anyhow::Result<Arc<dyn OcrEngine>> {
        match kind {
            #[cfg(windows)]
            "windows" => Ok(Arc::new(glint_ocr::WindowsOcr::new("auto")?)),
            _ => {
                let opts = glint_ocr::PaddleOptions::default();
                Ok(Arc::new(glint_ocr::PaddleOcr::load(&self.model_dir, opts)?))
            }
        }
    }
}
