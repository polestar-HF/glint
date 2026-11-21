//! ONNX Runtime plumbing: one process-wide environment, three sessions (det, rec, optional cls),
//! and thin typed `run_*` wrappers. Everything model-specific that does not need ORT lives in
//! `det.rs` / `rec.rs`.

use std::{
    path::{Path, PathBuf},
    sync::{Mutex, PoisonError},
};

use ort::{
    environment::Environment,
    ep,
    session::{builder::GraphOptimizationLevel, Session},
    value::Tensor,
};

use super::{rec::Dict, PaddleOptions};
use crate::{OcrError, Result};

/// The ORT environment is created once per process and kept alive for the process lifetime, so
/// loading, dropping (idle unload) and loading a `PaddleOcr` again never re-initializes the
/// runtime. A failed initialization is not cached: the caller may fix the path and retry.
static ORT_ENV: Mutex<Option<Environment>> = Mutex::new(None);

fn environment(opts: &PaddleOptions) -> Result<Environment> {
    let mut guard = ORT_ENV.lock().unwrap_or_else(PoisonError::into_inner);
    if let Some(env) = guard.as_ref() {
        return Ok(env.clone());
    }
    let env = init_builder(opts)?
        .with_name("glint-ocr")
        .with_telemetry(false)
        .build()
        .map_err(|e| OcrError::Load(format!("ONNX Runtime initialization failed: {e}")))?;
    *guard = Some(env.clone());
    Ok(env)
}

#[cfg(feature = "ort-dynamic")]
fn init_builder(opts: &PaddleOptions) -> Result<ort::environment::EnvironmentBuilder> {
    let path = resolve_dylib(opts)?;
    log::info!("loading ONNX Runtime from {}", path.display());
    ort::init_from(&path).map_err(|e| {
        OcrError::Load(format!(
            "cannot load ONNX Runtime from {}: {e}",
            path.display()
        ))
    })
}

#[cfg(not(feature = "ort-dynamic"))]
fn init_builder(_opts: &PaddleOptions) -> Result<ort::environment::EnvironmentBuilder> {
    Ok(ort::init())
}

/// Where to load the ONNX Runtime shared library from:
/// `PaddleOptions::ort_dylib_path`, then `ORT_DYLIB_PATH`, then next to the executable, then the
/// bare library name (left to the OS loader). An explicitly given path that does not exist is an
/// error and never falls back, because on Windows 11 `System32` holds an older, incompatible
/// `onnxruntime.dll`.
#[cfg(feature = "ort-dynamic")]
fn resolve_dylib(opts: &PaddleOptions) -> Result<PathBuf> {
    let exe_dir = std::env::current_exe()
        .ok()
        .and_then(|p| p.parent().map(Path::to_path_buf));
    let explicit = |p: PathBuf, what: &str| -> Result<PathBuf> {
        if p.is_file() {
            return Ok(p);
        }
        if p.is_relative() {
            if let Some(joined) = exe_dir.as_ref().map(|d| d.join(&p)).filter(|j| j.is_file()) {
                return Ok(joined);
            }
        }
        Err(OcrError::Load(format!(
            "ONNX Runtime library given by {what} not found: {}",
            p.display()
        )))
    };
    if let Some(p) = &opts.ort_dylib_path {
        return explicit(p.clone(), "PaddleOptions::ort_dylib_path");
    }
    if let Some(v) = std::env::var_os("ORT_DYLIB_PATH").filter(|v| !v.is_empty()) {
        return explicit(PathBuf::from(v), "ORT_DYLIB_PATH");
    }
    let name = format!(
        "{}onnxruntime{}",
        std::env::consts::DLL_PREFIX,
        std::env::consts::DLL_SUFFIX
    );
    if let Some(p) = exe_dir.map(|d| d.join(&name)).filter(|p| p.is_file()) {
        return Ok(p);
    }
    Ok(PathBuf::from(name))
}

fn load_err<E: std::fmt::Display>(what: &str) -> impl FnOnce(E) -> OcrError + '_ {
    move |e| OcrError::Load(format!("{what}: {e}"))
}

fn infer_err<E: std::fmt::Display>(e: E) -> OcrError {
    OcrError::Inference(e.to_string())
}

/// `min(6, cores / 2)`, at least 1.
/// Uses 2–4 threads on typical laptops and up to 6 on 12+ thread desktops (staying within
/// performance cores on hybrid CPUs while speeding up multi-line recognition).
pub(super) fn default_threads() -> usize {
    std::thread::available_parallelism().map_or(2, |n| (n.get() / 2).clamp(1, 6))
}

fn build_session_raw(
    env: &Environment,
    path: &Path,
    threads: usize,
    opt_level: GraphOptimizationLevel,
    save_optimized_to: Option<&Path>,
) -> Result<Session> {
    let what = format!("loading {}", path.display());
    let mut builder = Session::builder(env)
        .map_err(load_err(&what))?
        .with_optimization_level(opt_level)
        .map_err(load_err(&what))?
        .with_intra_threads(threads)
        .map_err(load_err(&what))?
        .with_inter_threads(1)
        .map_err(load_err(&what))?
        .with_parallel_execution(false)
        .map_err(load_err(&what))?
        // Shapes change on every call; the memory pattern cache and the arena would only hold
        // on to memory.
        .with_memory_pattern(false)
        .map_err(load_err(&what))?
        .with_execution_providers([ep::CPU::default().with_arena_allocator(false).build()])
        .map_err(load_err(&what))?;
    if let Some(out) = save_optimized_to {
        builder = builder
            .with_optimized_model_path(out)
            .map_err(load_err(&what))?;
    }
    builder.commit_from_file(path).map_err(load_err(&what))
}

/// Deterministic cache filename keyed by model stem, source length + mtime, ORT library length
/// and target architecture, so model or runtime upgrades invalidate the cache automatically.
fn optimized_cache_path(
    cache_dir: &Path,
    model_path: &Path,
    ort_dylib: Option<&Path>,
) -> Option<PathBuf> {
    let stem = model_path.file_stem()?.to_str()?;
    let meta = std::fs::metadata(model_path).ok()?;
    let len = meta.len();
    let mtime = meta
        .modified()
        .ok()
        .and_then(|t| t.duration_since(std::time::UNIX_EPOCH).ok())
        .map_or(0, |d| d.as_secs());
    let dylib_len = ort_dylib
        .and_then(|p| std::fs::metadata(p).ok())
        .map_or(0, |m| m.len());
    let arch = std::env::consts::ARCH;
    Some(cache_dir.join(format!(
        "{stem}-{arch}-{len:x}-{mtime:x}-{dylib_len:x}.opt.onnx"
    )))
}

fn build_session(
    env: &Environment,
    path: &Path,
    threads: usize,
    cache_dir: Option<&Path>,
    ort_dylib: Option<&Path>,
) -> Result<Session> {
    let cached = cache_dir.and_then(|dir| optimized_cache_path(dir, path, ort_dylib));
    if let Some(cp) = cached.as_deref().filter(|p| p.is_file()) {
        // Already graph-optimized: load with optimization disabled to skip repacking.
        match build_session_raw(env, cp, threads, GraphOptimizationLevel::Disable, None) {
            Ok(session) => {
                log::debug!("loaded cached optimized model {}", cp.display());
                return Ok(session);
            }
            Err(e) => {
                log::warn!(
                    "cached optimized model {} failed to load ({e}); rebuilding",
                    cp.display()
                );
                let _ = std::fs::remove_file(cp);
            }
        }
    }

    if let Some(cp) = cached.as_deref() {
        if let Some(dir) = cp.parent() {
            if std::fs::create_dir_all(dir).is_ok() {
                let tmp = cp.with_extension(format!("tmp.{}.onnx", std::process::id()));
                match build_session_raw(env, path, threads, GraphOptimizationLevel::All, Some(&tmp))
                {
                    Ok(session) => {
                        if let Err(e) = std::fs::rename(&tmp, cp) {
                            log::debug!("could not finalize model cache {}: {e}", cp.display());
                            let _ = std::fs::remove_file(&tmp);
                        }
                        return Ok(session);
                    }
                    Err(e) => {
                        let _ = std::fs::remove_file(&tmp);
                        log::warn!("optimized model cache write failed ({e}); loading uncached");
                    }
                }
            }
        }
    }

    // Highest level: basic + extended + layout (NCHWc) rewrites.
    build_session_raw(env, path, threads, GraphOptimizationLevel::All, None)
}

pub(super) struct Engine {
    det: Mutex<Session>,
    rec: Mutex<Session>,
    cls: Option<Mutex<Session>>,
    det_in: String,
    rec_in: String,
    cls_in: String,
    pub dict: Dict,
}

impl Engine {
    pub fn load(model_dir: &Path, opts: &PaddleOptions) -> Result<Self> {
        let need = |name: &str| -> Result<PathBuf> {
            let p = model_dir.join(name);
            if p.is_file() {
                Ok(p)
            } else {
                Err(OcrError::Load(format!(
                    "model file not found: {}",
                    p.display()
                )))
            }
        };
        let det_path = need("det.onnx")?;
        let rec_path = need("rec.onnx")?;
        let cls_path = if opts.use_angle_cls {
            Some(need("cls.onnx")?)
        } else {
            None
        };

        let env = environment(opts)?;
        let threads = opts.intra_threads.unwrap_or_else(default_threads).max(1);
        let cache_dir = opts.cache_dir.as_deref();
        let ort_dylib = opts.ort_dylib_path.as_deref();
        let det = build_session(&env, &det_path, threads, cache_dir, ort_dylib)?;
        let rec = build_session(&env, &rec_path, threads, cache_dir, ort_dylib)?;
        // The classifier is tiny; one thread is plenty and keeps the pools small.
        let cls = cls_path
            .map(|p| build_session(&env, &p, 1, cache_dir, ort_dylib))
            .transpose()?;

        let dict = load_dict(&rec, model_dir)?;
        let classes = rec
            .outputs()
            .first()
            .and_then(|o| o.dtype().tensor_shape().and_then(|s| s.last().copied()))
            .unwrap_or(-1);
        if classes > 0 && classes as usize != dict.classes() {
            return Err(OcrError::Load(format!(
                "rec.onnx outputs {classes} classes but the dictionary gives {} (blank + {} entries + space)",
                dict.classes(),
                dict.classes() - 2
            )));
        }
        let name_of = |s: &Session| {
            s.inputs()
                .first()
                .map(|i| i.name().to_string())
                .unwrap_or_else(|| "x".into())
        };
        Ok(Self {
            det_in: name_of(&det),
            rec_in: name_of(&rec),
            cls_in: cls.as_ref().map(name_of).unwrap_or_default(),
            det: Mutex::new(det),
            rec: Mutex::new(rec),
            cls: cls.map(Mutex::new),
            dict,
        })
    }

    /// Runs the detector on a `[1, 3, h, w]` input and returns the `h * w` probability map.
    pub fn run_det(&self, data: Vec<f32>, h: usize, w: usize) -> Result<Vec<f32>> {
        let input =
            Tensor::from_array(([1usize, 3, h, w], data.into_boxed_slice())).map_err(infer_err)?;
        let mut s = self.det.lock().unwrap_or_else(PoisonError::into_inner);
        let outputs = s
            .run(ort::inputs![self.det_in.as_str() => input])
            .map_err(infer_err)?;
        let (shape, out) = outputs[0].try_extract_tensor::<f32>().map_err(infer_err)?;
        if out.len() != h * w {
            return Err(OcrError::Inference(format!(
                "detector output {shape} does not match input {h}x{w}"
            )));
        }
        Ok(out.to_vec())
    }

    /// Runs the recognizer on `[n, 3, 48, w]` and hands `(probs, n, t, c)` to `f` while the
    /// output buffer is still alive (it can be tens of megabytes, so it is not copied).
    pub fn run_rec<R>(
        &self,
        data: Vec<f32>,
        n: usize,
        w: usize,
        f: impl FnOnce(&[f32], usize, usize, usize) -> R,
    ) -> Result<R> {
        let input = Tensor::from_array(([n, 3, super::rec::REC_H, w], data.into_boxed_slice()))
            .map_err(infer_err)?;
        let mut s = self.rec.lock().unwrap_or_else(PoisonError::into_inner);
        let outputs = s
            .run(ort::inputs![self.rec_in.as_str() => input])
            .map_err(infer_err)?;
        let (shape, out) = outputs[0].try_extract_tensor::<f32>().map_err(infer_err)?;
        if shape.len() != 3 || shape[0] as usize != n {
            return Err(OcrError::Inference(format!(
                "unexpected recognizer output shape {shape}"
            )));
        }
        let (t, c) = (shape[1] as usize, shape[2] as usize);
        Ok(f(out, n, t, c))
    }

    /// Runs the angle classifier on `[n, 3, 48, 192]`; returns `[p_0deg, p_180deg]` per item.
    pub fn run_cls(&self, data: Vec<f32>, n: usize) -> Result<Vec<[f32; 2]>> {
        let Some(cls) = &self.cls else {
            return Err(OcrError::Inference("angle classifier not loaded".into()));
        };
        let input = Tensor::from_array(([n, 3, 48usize, 192usize], data.into_boxed_slice()))
            .map_err(infer_err)?;
        let mut s = cls.lock().unwrap_or_else(PoisonError::into_inner);
        let outputs = s
            .run(ort::inputs![self.cls_in.as_str() => input])
            .map_err(infer_err)?;
        let (shape, out) = outputs[0].try_extract_tensor::<f32>().map_err(infer_err)?;
        if out.len() != n * 2 {
            return Err(OcrError::Inference(format!(
                "unexpected classifier output shape {shape}"
            )));
        }
        Ok(out.chunks_exact(2).map(|p| [p[0], p[1]]).collect())
    }

    pub fn has_cls(&self) -> bool {
        self.cls.is_some()
    }
}

/// Dictionary from the recognizer's `character` metadata (RapidOCR / PP-OCRv6 models embed it),
/// else from `dict.txt` (or `ppocr_keys_v1.txt`) in the model directory.
fn load_dict(rec: &Session, model_dir: &Path) -> Result<Dict> {
    if let Some(meta) = rec
        .metadata()
        .ok()
        .and_then(|m| m.custom("character"))
        .filter(|s| !s.is_empty())
    {
        return Dict::from_metadata(&meta);
    }
    for name in ["dict.txt", "ppocr_keys_v1.txt"] {
        let p = model_dir.join(name);
        if p.is_file() {
            let text = std::fs::read_to_string(&p)
                .map_err(load_err(&format!("reading {}", p.display())))?;
            return Dict::from_file_text(&text);
        }
    }
    Err(OcrError::Load(
        "rec.onnx has no embedded `character` dictionary and no dict.txt was found in the model directory".into(),
    ))
}
