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
    /// Absolute path of the bundled onnxruntime.dll (Windows 11 ships an older one in System32).
    ort_dylib: PathBuf,
    slot: Mutex<Option<Loaded>>,
    /// Serializes loads so a preload and a real request never load twice.
    load_lock: Mutex<()>,
    /// When the last load finished and how long it took (performance stats).
    last_load: Mutex<Option<(Instant, Duration)>>,
}

impl OcrService {
    pub fn new(model_dir: PathBuf, ort_dylib: PathBuf) -> Self {
        Self {
            model_dir,
            ort_dylib,
            slot: Mutex::new(None),
            load_lock: Mutex::new(()),
            last_load: Mutex::new(None),
        }
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
        let took = started.elapsed();
        log::info!("ocr engine '{kind}' loaded in {took:?}");
        *self.last_load.lock().unwrap_or_else(|p| p.into_inner()) = Some((Instant::now(), took));
        *self.slot.lock().unwrap_or_else(|p| p.into_inner()) = Some(Loaded {
            kind: kind.to_string(),
            engine: engine.clone(),
            last_used: Instant::now(),
        });
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

    /// The load time, if the engine was (re)loaded after `since` — i.e. this capture paid for
    /// a cold start, even when the load ran in the hotkey-time preload.
    pub fn loaded_since(&self, since: Instant) -> Option<Duration> {
        let last = *self.last_load.lock().unwrap_or_else(|p| p.into_inner());
        last.filter(|(at, _)| *at >= since).map(|(_, took)| took)
    }

    /// Drop the engine if unused for `idle`. Called periodically.
    pub fn unload_if_idle(&self, idle: Duration) {
        let mut slot = self.slot.lock().unwrap_or_else(|p| p.into_inner());
        if slot.as_ref().is_some_and(|l| l.last_used.elapsed() >= idle) {
            log::info!("unloading idle ocr engine");
            *slot = None;
            drop(slot);
            // Hand freed ONNX buffers back to the OS so idle RSS actually drops.
            glint_ocr::trim_process_memory();
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
                let opts = glint_ocr::PaddleOptions {
                    ort_dylib_path: Some(self.ort_dylib.clone()),
                    ..Default::default()
                };
                match glint_ocr::PaddleOcr::load(&self.model_dir, opts) {
                    Ok(engine) => Ok(Arc::new(engine)),
                    Err(e) => self.fallback(e),
                }
            }
        }
    }

    /// A broken PaddleOCR install (missing runtime DLL, damaged model) degrades to the
    /// built-in Windows OCR instead of failing every capture. `OcrResult::engine` tells
    /// the popup which engine actually ran.
    fn fallback(&self, err: glint_ocr::OcrError) -> anyhow::Result<Arc<dyn OcrEngine>> {
        log::error!("paddle ocr load failed: {err}");
        #[cfg(windows)]
        if let Ok(engine) = glint_ocr::WindowsOcr::new("auto") {
            log::warn!("falling back to Windows OCR");
            return Ok(Arc::new(engine));
        }
        Err(err.into())
    }
}
