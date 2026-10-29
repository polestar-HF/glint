//! Runs PaddleOcr on one image and prints the lines, the merged text and the timings.
//!
//! ```text
//! ORT_DYLIB_PATH=models/ort/libonnxruntime.so \
//!   cargo run --release -p glint-ocr --example ocr -- shot.png [models/v6-small] [runs]
//! ```

use std::path::PathBuf;

use glint_ocr::{OcrEngine, OcrOptions, PaddleOcr, PaddleOptions};

fn main() {
    let mut args = std::env::args().skip(1);
    let Some(image_path) = args.next() else {
        eprintln!("usage: ocr <image> [model_dir] [runs]");
        std::process::exit(2);
    };
    let model_dir = PathBuf::from(args.next().unwrap_or_else(|| "models/v6-small".into()));
    let runs: usize = args.next().and_then(|r| r.parse().ok()).unwrap_or(3);

    let img = image::open(&image_path).expect("cannot read image").to_rgba8();
    let t = std::time::Instant::now();
    let ocr = PaddleOcr::load(&model_dir, PaddleOptions::default()).expect("cannot load models");
    println!("load: {:?}", t.elapsed());
    let opts = OcrOptions::default();
    for i in 0..runs {
        let (res, tm) = ocr.recognize_timed(&img, &opts).expect("ocr failed");
        println!(
            "run {i}: total {:?} (prepare {:?}, det {:?}, rec {:?}, post {:?}), {} boxes",
            tm.total, tm.prepare, tm.detect, tm.recognize, tm.postprocess, tm.boxes
        );
        if i + 1 == runs {
            for l in &res.lines {
                println!("  [{:.2}] {:?} @ {:?}", l.confidence, l.text, l.bbox.map(|v| v.round() as i32));
            }
            println!("lang: {:?}\n---\n{}\n---", res.detected_lang, res.text);
        }
    }
    let _ = ocr.name();
}
