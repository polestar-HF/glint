//! `ort-dynamic` library lookup rules. These run without models or an ONNX Runtime: they only
//! exercise the error paths, which must be reported as `OcrError::Load`, never panic, and never
//! fall back to some other library. (Own test binary, so the process-wide ORT environment is
//! untouched by other tests.)
#![cfg(feature = "ort-dynamic")]

use std::path::PathBuf;

use glint_ocr::{OcrError, PaddleOcr, PaddleOptions};

fn model_dir_with_placeholder_files() -> PathBuf {
    let dir = std::env::temp_dir().join(format!("glint-ocr-dylib-test-{}", std::process::id()));
    std::fs::create_dir_all(&dir).expect("temp dir");
    for f in ["det.onnx", "rec.onnx"] {
        std::fs::write(dir.join(f), b"not a model").expect("placeholder");
    }
    dir
}

#[test]
fn missing_explicit_dylib_path_is_a_load_error_that_names_the_option() {
    let dir = model_dir_with_placeholder_files();
    let opts = PaddleOptions {
        ort_dylib_path: Some(dir.join("no-such-dir/onnxruntime.dll")),
        ..PaddleOptions::default()
    };
    match PaddleOcr::load(&dir, opts) {
        Err(OcrError::Load(msg)) => assert!(
            msg.contains("ort_dylib_path") && msg.contains("not found"),
            "{msg}"
        ),
        Err(e) => panic!("wrong error: {e}"),
        Ok(_) => panic!("must not load without a runtime"),
    }
}

#[test]
fn file_that_is_not_a_runtime_is_a_load_error() {
    let dir = model_dir_with_placeholder_files();
    // `det.onnx` exists but is certainly not an ONNX Runtime library.
    let opts = PaddleOptions {
        ort_dylib_path: Some(dir.join("det.onnx")),
        ..PaddleOptions::default()
    };
    match PaddleOcr::load(&dir, opts) {
        Err(OcrError::Load(msg)) => assert!(msg.contains("cannot load ONNX Runtime"), "{msg}"),
        Err(e) => panic!("wrong error: {e}"),
        Ok(_) => panic!("must not load"),
    }
}
