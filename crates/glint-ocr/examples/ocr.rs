//! Runs PaddleOcr on one image and prints the lines, the merged text and the timings.
//!
//! ```text
//! ORT_DYLIB_PATH=models/ort/libonnxruntime.so \
//!   cargo run --release -p glint-ocr --example ocr -- shot.png [models/v6-small] [runs]
//! ```

use std::path::PathBuf;

use glint_ocr::{OcrEngine, OcrOptions, PaddleOcr, PaddleOptions};

/// Resident / peak resident memory in MB (Linux only; 0 elsewhere).
fn rss(field: &str) -> f64 {
    std::fs::read_to_string("/proc/self/status")
        .ok()
        .and_then(|s| s.lines().find(|l| l.starts_with(field)).map(str::to_owned))
        .and_then(|l| {
            l.split_whitespace()
                .nth(1)
                .and_then(|v| v.parse::<f64>().ok())
        })
        .map_or(0.0, |kb| kb / 1024.0)
}

fn main() {
    let mut args = std::env::args().skip(1);
    let Some(image_path) = args.next() else {
        eprintln!("usage: ocr <image> [model_dir] [runs]");
        std::process::exit(2);
    };
    let model_dir = PathBuf::from(args.next().unwrap_or_else(|| "models/v6-small".into()));
    let runs: usize = args.next().and_then(|r| r.parse().ok()).unwrap_or(3);

    let img = image::open(&image_path)
        .expect("cannot read image")
        .to_rgba8();
    let t = std::time::Instant::now();
    // Experiments: GLINT_THREADS, GLINT_BATCH, GLINT_BATCH_AREA, GLINT_MINW override the defaults.
    let env_num = |k: &str| std::env::var(k).ok().and_then(|v| v.parse::<usize>().ok());
    let defaults = PaddleOptions::default();
    let popts = PaddleOptions {
        intra_threads: env_num("GLINT_THREADS"),
        rec_batch: env_num("GLINT_BATCH").unwrap_or(defaults.rec_batch),
        rec_batch_max_area: env_num("GLINT_BATCH_AREA").unwrap_or(defaults.rec_batch_max_area),
        rec_min_width: env_num("GLINT_MINW").unwrap_or(defaults.rec_min_width),
        ..defaults
    };
    let ocr = PaddleOcr::load(&model_dir, popts).expect("cannot load models");
    println!("load: {:?}, RSS {:.0} MB", t.elapsed(), rss("VmRSS"));
    let opts = OcrOptions::default();
    for i in 0..runs {
        let (res, tm) = ocr.recognize_timed(&img, &opts).expect("ocr failed");
        println!(
            "run {i}: total {:?} (prepare {:?}, det {:?}, rec {:?}, post {:?}), {} boxes",
            tm.total, tm.prepare, tm.detect, tm.recognize, tm.postprocess, tm.boxes
        );
        if i + 1 == runs {
            for l in &res.lines {
                println!(
                    "  [{:.2}] {:?} @ {:?}",
                    l.confidence,
                    l.text,
                    l.bbox.map(|v| v.round() as i32)
                );
            }
            println!("lang: {:?}\n---\n{}\n---", res.detected_lang, res.text);
        }
    }
    println!("RSS {:.0} MB, peak {:.0} MB", rss("VmRSS"), rss("VmHWM"));
    let _ = ocr.name();
    if std::env::var_os("GLINT_DROP").is_some() {
        drop(ocr);
        println!("after drop: RSS {:.0} MB", rss("VmRSS"));
        #[cfg(target_os = "linux")]
        {
            extern "C" {
                fn malloc_trim(pad: usize) -> i32;
            }
            // SAFETY: plain glibc call without arguments that carry pointers.
            unsafe { malloc_trim(0) };
            println!("after malloc_trim: RSS {:.0} MB", rss("VmRSS"));
            if let Ok(s) = std::fs::read_to_string("/proc/self/smaps_rollup") {
                for l in s.lines().filter(|l| {
                    l.starts_with("Rss")
                        || l.starts_with("Pss_Anon")
                        || l.starts_with("Pss_File")
                        || l.starts_with("Private_Dirty")
                        || l.starts_with("Anonymous")
                }) {
                    println!("  {l}");
                }
            }
        }
    }
}
