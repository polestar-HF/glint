//! Accuracy and latency checks against the PNG fixtures in `tests/fixtures/`.
//!
//! These need the PP-OCRv6 models and an ONNX Runtime shared library, so they are `#[ignore]`d
//! and `cargo test` passes without them. Run with
//!
//! ```text
//! GLINT_OCR_MODELS=models/v6-small ORT_DYLIB_PATH=models/ort/libonnxruntime.so \
//!   cargo test -p glint-ocr --release -- --ignored --nocapture
//! ```
//!
//! `GLINT_OCR_MODELS` defaults to `<workspace>/models/v6-small`. The ONNX Runtime library comes
//! from `GLINT_ORT_DYLIB` or `ORT_DYLIB_PATH` (or sits next to the test executable).
//! `GLINT_FIXTURE_RUNS` sets the number of timed warm runs (default 7).

use std::{
    collections::BTreeMap,
    path::PathBuf,
    time::{Duration, Instant},
};

use glint_ocr::{OcrEngine, OcrOptions, OcrResult, PaddleOcr, PaddleOptions};

fn manifest_dir() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
}

fn model_dir() -> PathBuf {
    std::env::var_os("GLINT_OCR_MODELS")
        .map(PathBuf::from)
        .unwrap_or_else(|| manifest_dir().join("../../models/v6-small"))
}

fn paddle_options() -> PaddleOptions {
    PaddleOptions {
        ort_dylib_path: std::env::var_os("GLINT_ORT_DYLIB").map(PathBuf::from),
        intra_threads: std::env::var("GLINT_THREADS")
            .ok()
            .and_then(|v| v.parse().ok()),
        ..PaddleOptions::default()
    }
}

fn load() -> PaddleOcr {
    let dir = model_dir();
    PaddleOcr::load(&dir, paddle_options()).unwrap_or_else(|e| {
        panic!(
            "cannot load models from {}: {e}\n(see the header of tests/fixtures.rs)",
            dir.display()
        )
    })
}

struct Case {
    expected: String,
    lang: String,
}

fn cases() -> BTreeMap<String, Case> {
    let text = std::fs::read_to_string(manifest_dir().join("tests/fixtures/expected.json"))
        .expect("expected.json");
    let json: serde_json::Value = serde_json::from_str(&text).expect("valid json");
    json.as_object()
        .expect("object")
        .iter()
        .map(|(k, v)| {
            (
                k.clone(),
                Case {
                    expected: v["expected"].as_str().expect("expected").to_string(),
                    lang: v["lang"].as_str().expect("lang").to_string(),
                },
            )
        })
        .collect()
}

fn image(name: &str) -> image::RgbaImage {
    image::open(manifest_dir().join(format!("tests/fixtures/{name}.png")))
        .expect("fixture png")
        .to_rgba8()
}

fn levenshtein(a: &[char], b: &[char]) -> usize {
    let mut prev: Vec<usize> = (0..=b.len()).collect();
    for (i, ca) in a.iter().enumerate() {
        let mut cur = vec![i + 1];
        for (j, cb) in b.iter().enumerate() {
            cur.push(
                (prev[j + 1] + 1)
                    .min(cur[j] + 1)
                    .min(prev[j] + usize::from(ca != cb)),
            );
        }
        prev = cur;
    }
    prev[b.len()]
}

/// Character error rate ignoring all whitespace (layout is checked separately).
fn cer(reference: &str, hyp: &str) -> f32 {
    let strip = |s: &str| s.chars().filter(|c| !c.is_whitespace()).collect::<Vec<_>>();
    let (r, h) = (strip(reference), strip(hyp));
    levenshtein(&r, &h) as f32 / r.len().max(1) as f32
}

fn rss_mb(field: &str) -> f64 {
    let s = std::fs::read_to_string("/proc/self/status").unwrap_or_default();
    s.lines()
        .find(|l| l.starts_with(field))
        .and_then(|l| l.split_whitespace().nth(1))
        .and_then(|v| v.parse::<f64>().ok())
        .map_or(0.0, |kb| kb / 1024.0)
}

fn paragraph_count(r: &OcrResult) -> usize {
    r.text.split('\n').filter(|l| !l.is_empty()).count()
}

#[test]
#[ignore = "needs models and ONNX Runtime (see tests/fixtures.rs)"]
fn fixtures_accuracy_and_latency() {
    let runs: usize = std::env::var("GLINT_FIXTURE_RUNS")
        .ok()
        .and_then(|v| v.parse().ok())
        .unwrap_or(7);
    let rss0 = rss_mb("VmRSS");
    let t = Instant::now();
    let ocr = load();
    let load_time = t.elapsed();
    let rss_loaded = rss_mb("VmRSS");
    let opts = OcrOptions::default();

    println!("\nload {load_time:?}; RSS {rss0:.0} -> {rss_loaded:.0} MB");
    println!(
        "{:<14} {:>6} {:>7} {:>9} {:>9} {:>9} {:>9} {:>6}",
        "fixture", "CER", "lines", "cold ms", "warm min", "warm med", "det/rec", "lang"
    );
    let mut failures = Vec::new();
    let mut first = true;
    for (name, case) in cases() {
        let img = image(&name);
        let t = Instant::now();
        let (res, _) = ocr.recognize_timed(&img, &opts).expect("recognize");
        let cold = t.elapsed();
        let mut times = Vec::new();
        let mut det_rec = (Duration::MAX, Duration::MAX);
        for _ in 0..runs {
            let (_, tm) = ocr.recognize_timed(&img, &opts).expect("recognize");
            times.push(tm.total);
            det_rec = (det_rec.0.min(tm.detect), det_rec.1.min(tm.recognize));
        }
        times.sort();
        let e = cer(&case.expected, &res.text);
        println!(
            "{:<14} {:>6.3} {:>7} {:>9.0} {:>9.0} {:>9.0} {:>4.0}/{:<4.0} {:>6}{}",
            name,
            e,
            res.lines.len(),
            cold.as_secs_f64() * 1e3,
            times[0].as_secs_f64() * 1e3,
            times[times.len() / 2].as_secs_f64() * 1e3,
            det_rec.0.as_secs_f64() * 1e3,
            det_rec.1.as_secs_f64() * 1e3,
            res.detected_lang.as_deref().unwrap_or("-"),
            if first {
                "   (first fixture also pays the one-time ORT warm-up)"
            } else {
                ""
            }
        );
        first = false;
        if e > 0.02 {
            failures.push(format!(
                "{name}: CER {e:.3} > 0.02\n  expected: {:?}\n  got:      {:?}",
                case.expected, res.text
            ));
        }
        if res.detected_lang.as_deref() != Some(case.lang.as_str()) {
            failures.push(format!(
                "{name}: detected {:?}, expected {}",
                res.detected_lang, case.lang
            ));
        }
        // Layout expectations per fixture.
        let paragraphs = paragraph_count(&res);
        let want = case.expected.split('\n').count();
        if paragraphs != want {
            failures.push(format!(
                "{name}: {paragraphs} paragraphs, expected {want}: {:?}",
                res.text
            ));
        }
    }
    println!(
        "RSS after run {:.0} MB (peak {:.0} MB)",
        rss_mb("VmRSS"),
        rss_mb("VmHWM")
    );
    assert!(failures.is_empty(), "\n{}", failures.join("\n"));
}

#[test]
#[ignore = "needs models and ONNX Runtime (see tests/fixtures.rs)"]
fn exact_text_on_clean_fixtures() {
    let ocr = load();
    let opts = OcrOptions::default();
    for (name, want) in [
        (
            "en_paragraph",
            "Glint recognizes text on your screen in a fraction of a second. Select any region with the mouse, and the translation appears right next to it, so you never have to switch windows or retype a single word.",
        ),
        ("dark_ui", "General\nAppearance\nKeyboard Shortcuts\nTranslation Engines\nAbout Glint"),
        (
            "two_column",
            "Fast offline recognition keeps every screenshot on your own computer, with no upload and no account.\nTranslate with the engine you trust: a free web service, your own API key, or a local model running on this PC.",
        ),
    ] {
        let got = ocr.recognize(&image(name), &opts).expect("recognize").text;
        assert_eq!(got, want, "{name}");
    }
}

#[test]
#[ignore = "needs models and ONNX Runtime (see tests/fixtures.rs)"]
fn merge_lines_false_keeps_visual_lines() {
    let ocr = load();
    let opts = OcrOptions {
        merge_lines: false,
        ..OcrOptions::default()
    };
    let res = ocr
        .recognize(&image("en_paragraph"), &opts)
        .expect("recognize");
    assert_eq!(res.lines.len(), 4);
    assert_eq!(res.text.split('\n').count(), 4);
    for l in &res.lines {
        let [x, y, w, h] = l.bbox;
        assert!(
            x >= 0.0 && y >= 0.0 && w > 20.0 && h > 8.0 && x + w <= 600.0 && y + h <= 112.0,
            "{l:?}"
        );
        assert!(l.confidence > 0.9);
    }
}

#[test]
#[ignore = "needs models and ONNX Runtime (see tests/fixtures.rs)"]
fn blank_and_tiny_images_do_not_fail() {
    let ocr = load();
    let opts = OcrOptions::default();
    for (w, h) in [(1, 1), (2, 40), (40, 2), (30, 12), (300, 120)] {
        let img = image::RgbaImage::from_pixel(w, h, image::Rgba([255, 255, 255, 255]));
        let r = ocr.recognize(&img, &opts).expect("recognize");
        assert!(r.lines.is_empty() && r.text.is_empty(), "{w}x{h}: {r:?}");
        assert_eq!(r.engine, "paddle");
    }
}

#[test]
#[ignore = "needs models and ONNX Runtime (see tests/fixtures.rs)"]
fn reload_after_drop_and_concurrent_use() {
    let opts = OcrOptions::default();
    let img = image("dark_ui");
    let expect = "General\nAppearance\nKeyboard Shortcuts\nTranslation Engines\nAbout Glint";
    // Idle unload: drop the engine, load again; the ORT environment is reused.
    for _ in 0..2 {
        let ocr = load();
        assert_eq!(ocr.recognize(&img, &opts).unwrap().text, expect);
        drop(ocr);
    }
    // Shared between threads.
    let ocr = std::sync::Arc::new(load());
    let handles: Vec<_> = (0..3)
        .map(|_| {
            let (ocr, img, opts) = (ocr.clone(), img.clone(), opts.clone());
            std::thread::spawn(move || ocr.recognize(&img, &opts).unwrap().text)
        })
        .collect();
    for h in handles {
        assert_eq!(h.join().unwrap(), expect);
    }
}

#[test]
#[cfg(any(feature = "ort-dynamic", feature = "ort-static"))]
fn load_errors_are_reported_not_panics() {
    // These do not need ONNX Runtime: the files are checked first.
    let err = PaddleOcr::load(
        std::path::Path::new("/definitely/not/here"),
        PaddleOptions::default(),
    )
    .err()
    .expect("error");
    assert!(err.to_string().contains("det.onnx"), "{err}");
}

/// Experiment behind the `rec_batch` default: min-of-N latency per batch size, interleaved so
/// that background load on the machine hits every setting equally.
#[test]
#[ignore = "benchmark; needs models and ONNX Runtime (see tests/fixtures.rs)"]
fn bench_rec_batching() {
    let engines: Vec<(usize, PaddleOcr)> = [1usize, 2, 4]
        .iter()
        .map(|&b| {
            let opts = PaddleOptions {
                rec_batch: b,
                ..paddle_options()
            };
            (b, PaddleOcr::load(&model_dir(), opts).expect("load"))
        })
        .collect();
    let opts = OcrOptions::default();
    for name in ["en_paragraph", "zh_paragraph", "two_column", "dark_ui"] {
        let img = image(name);
        let mut best = vec![Duration::MAX; engines.len()];
        for _ in 0..12 {
            for (k, (_, e)) in engines.iter().enumerate() {
                let (_, tm) = e.recognize_timed(&img, &opts).expect("recognize");
                best[k] = best[k].min(tm.total);
            }
        }
        let row: Vec<String> = engines
            .iter()
            .zip(&best)
            .map(|((b, _), t)| format!("batch {b}: {:.0} ms", t.as_secs_f64() * 1e3))
            .collect();
        println!("{name:<14} {}", row.join("   "));
    }
}

#[test]
#[ignore = "needs models (incl. cls.onnx) and ONNX Runtime (see tests/fixtures.rs)"]
fn angle_classifier_fixes_upside_down_text_and_keeps_upright_text() {
    if !model_dir().join("cls.onnx").is_file() {
        eprintln!("cls.onnx not present, skipping");
        return;
    }
    let ocr = PaddleOcr::load(
        &model_dir(),
        PaddleOptions {
            use_angle_cls: true,
            ..paddle_options()
        },
    )
    .expect("load");
    let opts = OcrOptions {
        merge_lines: false,
        ..OcrOptions::default()
    };
    let labels = [
        "General",
        "Appearance",
        "Keyboard Shortcuts",
        "Translation Engines",
        "About Glint",
    ];

    let upright = ocr.recognize(&image("dark_ui"), &opts).expect("recognize");
    assert_eq!(upright.text, labels.join("\n"));

    let flipped = image::imageops::rotate180(&image("dark_ui"));
    let res = ocr.recognize(&flipped, &opts).expect("recognize");
    let got: Vec<&str> = res.lines.iter().map(|l| l.text.as_str()).collect();
    for l in labels {
        assert!(got.contains(&l), "{l:?} missing from {got:?}");
    }
}

#[test]
#[ignore = "needs models and ONNX Runtime (see tests/fixtures.rs)"]
fn optimized_model_cache_reloads_and_recovers_from_corruption() {
    let dir = std::env::temp_dir().join(format!("glint-ocr-cache-test-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    let opts = || PaddleOptions {
        cache_dir: Some(dir.clone()),
        ..paddle_options()
    };

    // 1. Cold load populates cache.
    let t0 = Instant::now();
    let ocr1 = PaddleOcr::load(&model_dir(), opts()).expect("cold cached load");
    let cold_ms = t0.elapsed().as_secs_f64() * 1e3;
    let img = image("en_paragraph");
    let r1 = ocr1.recognize(&img, &OcrOptions::default()).expect("rec 1");
    drop(ocr1);

    let cached_files: Vec<PathBuf> = std::fs::read_dir(&dir)
        .expect("cache dir created")
        .filter_map(|e| e.ok().map(|e| e.path()))
        .filter(|p| p.extension().is_some_and(|ext| ext == "onnx"))
        .collect();
    assert_eq!(
        cached_files.len(),
        2,
        "expected det + rec cached models, got {cached_files:?}"
    );

    // 2. Warm reload hits cache, preserves embedded dictionary and produces identical output.
    let t1 = Instant::now();
    let ocr2 = PaddleOcr::load(&model_dir(), opts()).expect("warm cached load");
    let warm_ms = t1.elapsed().as_secs_f64() * 1e3;
    let r2 = ocr2.recognize(&img, &OcrOptions::default()).expect("rec 2");
    assert_eq!(r1.text, r2.text, "cached model must produce identical OCR");
    drop(ocr2);
    println!("model load: uncached {cold_ms:.1} ms -> cached {warm_ms:.1} ms");

    // 3. Corrupted cache file is automatically discarded and rebuilt.
    std::fs::write(&cached_files[0], b"not an onnx model").expect("corrupt cache");
    let ocr3 = PaddleOcr::load(&model_dir(), opts()).expect("recover from corrupt cache");
    let r3 = ocr3.recognize(&img, &OcrOptions::default()).expect("rec 3");
    assert_eq!(r1.text, r3.text);

    let _ = std::fs::remove_dir_all(&dir);
}
