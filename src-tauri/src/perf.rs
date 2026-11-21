//! Rolling performance statistics over the last sessions, persisted next to the config so a
//! few days of real use can be summarized (Settings → 关于) and shared (`get_diagnostics`).

use std::collections::VecDeque;
use std::fmt::Write as _;
use std::path::PathBuf;
use std::sync::Mutex;
use std::time::{SystemTime, UNIX_EPOCH};

use serde::{Deserialize, Serialize};

const CAPACITY: usize = 300;

/// One capture (or retranslation) session. Times are milliseconds.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct Sample {
    pub at: u64,
    pub hotkey_to_overlay: Option<f64>,
    pub capture: Option<f64>,
    pub overlay: Option<f64>,
    /// Recognition time, excluding any model load.
    pub ocr: Option<f64>,
    /// Actual wall-clock wait after selection finished (includes only the portion of model
    /// loading that did not finish during the hotkey-time preload).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub ocr_wall: Option<f64>,
    /// The model was loaded for this capture (first use or after idle unload).
    pub ocr_cold: bool,
    pub ocr_load: Option<f64>,
    pub det: Option<f64>,
    pub rec: Option<f64>,
    pub engines: Vec<EngineSample>,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct EngineSample {
    pub engine_id: String,
    pub engine_name: String,
    pub elapsed: Option<f64>,
    pub first_token: Option<f64>,
    pub error: bool,
    /// Stable error category (`rate_limited`, `network`, `timeout`, `auth`, ...).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub error_code: Option<String>,
}

#[derive(Debug, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
struct Store {
    since: u64,
    samples: VecDeque<Sample>,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Metric {
    pub key: &'static str,
    pub n: u32,
    pub p50: f64,
    pub p90: f64,
    pub max: f64,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct EngineStat {
    pub engine_id: String,
    pub engine_name: String,
    pub n: u32,
    pub errors: u32,
    /// Subset of `errors` caused by rate limiting / cool-down skips.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub rate_limited: Option<u32>,
    pub p50: f64,
    pub p90: f64,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub first_token_p50: Option<f64>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct MonitorInfo {
    pub width: u32,
    pub height: u32,
    pub scale: f32,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SystemInfo {
    pub app: String,
    pub os: String,
    pub cpu_threads: u32,
    pub webview: String,
    pub monitors: Vec<MonitorInfo>,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct PerfStats {
    pub sessions: u32,
    pub since: String,
    pub metrics: Vec<Metric>,
    pub engines: Vec<EngineStat>,
    pub system: SystemInfo,
}

pub struct Perf {
    path: PathBuf,
    store: Mutex<Store>,
    monitors: Mutex<Vec<MonitorInfo>>,
}

impl Perf {
    pub fn load(path: PathBuf) -> Self {
        let store = std::fs::read(&path)
            .ok()
            .and_then(|raw| serde_json::from_slice::<Store>(&raw).ok())
            .unwrap_or_else(|| Store {
                since: now(),
                samples: VecDeque::new(),
            });
        Self {
            path,
            store: Mutex::new(store),
            monitors: Mutex::new(Vec::new()),
        }
    }

    pub fn record(&self, mut sample: Sample) {
        sample.at = now();
        let mut store = self.store.lock().unwrap_or_else(|p| p.into_inner());
        if store.samples.len() >= CAPACITY {
            store.samples.pop_front();
        }
        store.samples.push_back(sample);
        self.persist(&store);
    }

    pub fn reset(&self) {
        let mut store = self.store.lock().unwrap_or_else(|p| p.into_inner());
        *store = Store {
            since: now(),
            samples: VecDeque::new(),
        };
        self.persist(&store);
    }

    pub fn set_monitors(&self, monitors: Vec<MonitorInfo>) {
        *self.monitors.lock().unwrap_or_else(|p| p.into_inner()) = monitors;
    }

    pub fn monitors(&self) -> Vec<MonitorInfo> {
        self.monitors
            .lock()
            .unwrap_or_else(|p| p.into_inner())
            .clone()
    }

    pub fn stats(&self, system: SystemInfo) -> PerfStats {
        let store = self.store.lock().unwrap_or_else(|p| p.into_inner());
        summarize(&store, system)
    }

    /// Small file (≤ 300 samples); written in place of the old one so a crash never leaves
    /// a truncated store.
    fn persist(&self, store: &Store) {
        let Ok(json) = serde_json::to_vec(store) else {
            return;
        };
        if let Some(dir) = self.path.parent() {
            let _ = std::fs::create_dir_all(dir);
        }
        let tmp = self.path.with_extension("json.tmp");
        if let Err(e) = std::fs::write(&tmp, json).and_then(|()| std::fs::rename(&tmp, &self.path))
        {
            log::warn!("perf stats not saved: {e}");
        }
    }
}

fn now() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0)
}

fn summarize(store: &Store, system: SystemInfo) -> PerfStats {
    type Pick = fn(&Sample) -> Option<f64>;
    let picks: [(&'static str, Pick); 8] = [
        ("hotkeyToOverlay", |s| s.hotkey_to_overlay),
        ("capture", |s| s.capture),
        ("overlay", |s| s.overlay),
        ("ocr", |s| s.ocr.filter(|_| !s.ocr_cold)),
        ("ocrCold", |s| {
            if !s.ocr_cold {
                return None;
            }
            // Prefer the actual post-selection wall time (excludes preload hidden behind drag);
            // fall back to `ocr + ocr_load` for older persisted samples.
            s.ocr_wall
                .or_else(|| s.ocr.map(|ocr| ocr + s.ocr_load.unwrap_or(0.0)))
        }),
        ("ocrLoad", |s| s.ocr_load),
        ("det", |s| s.det),
        ("rec", |s| s.rec),
    ];
    let metrics = picks
        .iter()
        .filter_map(|(key, pick)| {
            let values: Vec<f64> = store.samples.iter().filter_map(pick).collect();
            let (p50, p90, max) = percentiles(values.clone())?;
            Some(Metric {
                key,
                n: values.len() as u32,
                p50,
                p90,
                max,
            })
        })
        .collect();

    // Engines in order of first appearance, so the table matches the user's engine order.
    #[derive(Default)]
    struct Acc {
        id: String,
        name: String,
        elapsed: Vec<f64>,
        first: Vec<f64>,
        errors: u32,
        rate_limited: u32,
    }
    let mut accs: Vec<Acc> = Vec::new();
    for e in store.samples.iter().flat_map(|s| &s.engines) {
        let idx = match accs.iter().position(|a| a.id == e.engine_id) {
            Some(i) => i,
            None => {
                accs.push(Acc {
                    id: e.engine_id.clone(),
                    ..Default::default()
                });
                accs.len() - 1
            }
        };
        let acc = &mut accs[idx];
        acc.name = e.engine_name.clone(); // latest name wins (services can be renamed)
        if e.error {
            acc.errors += 1;
            if e.error_code.as_deref() == Some("rate_limited") {
                acc.rate_limited += 1;
            }
        } else if let Some(ms) = e.elapsed {
            acc.elapsed.push(ms);
        }
        if let Some(ms) = e.first_token {
            acc.first.push(ms);
        }
    }
    let engines = accs
        .into_iter()
        .map(|a| {
            let (p50, p90, _) = percentiles(a.elapsed.clone()).unwrap_or((0.0, 0.0, 0.0));
            EngineStat {
                engine_id: a.id,
                engine_name: a.name,
                n: a.elapsed.len() as u32,
                errors: a.errors,
                rate_limited: (a.rate_limited > 0).then_some(a.rate_limited),
                p50,
                p90,
                first_token_p50: percentiles(a.first).map(|(p50, _, _)| p50),
            }
        })
        .collect();

    PerfStats {
        sessions: store.samples.len() as u32,
        since: rfc3339(store.since),
        metrics,
        engines,
        system,
    }
}

/// Nearest-rank p50 / p90 and max; `None` for no data.
fn percentiles(mut values: Vec<f64>) -> Option<(f64, f64, f64)> {
    values.retain(|v| v.is_finite());
    if values.is_empty() {
        return None;
    }
    values.sort_by(f64::total_cmp);
    let rank = |p: f64| {
        let i = ((p * values.len() as f64).ceil() as usize).clamp(1, values.len()) - 1;
        values[i]
    };
    Some((rank(0.5), rank(0.9), values[values.len() - 1]))
}

/// UTC timestamp without pulling in a date crate (civil-from-days, Howard Hinnant).
fn rfc3339(secs: u64) -> String {
    let days = (secs / 86_400) as i64;
    let rem = secs % 86_400;
    let z = days + 719_468;
    let era = z.div_euclid(146_097);
    let doe = z - era * 146_097;
    let yoe = (doe - doe / 1_460 + doe / 36_524 - doe / 146_096) / 365;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let d = doy - (153 * mp + 2) / 5 + 1;
    let m = if mp < 10 { mp + 3 } else { mp - 9 };
    let y = yoe + era * 400 + i64::from(m <= 2);
    format!(
        "{y:04}-{m:02}-{d:02}T{:02}:{:02}:{:02}Z",
        rem / 3_600,
        rem % 3_600 / 60,
        rem % 60
    )
}

/// Plain-text report for bug reports and tuning.
pub fn diagnostics(stats: &PerfStats) -> String {
    fn label(key: &str) -> &str {
        match key {
            "hotkeyToOverlay" => "按键到选区出现",
            "capture" => "截屏",
            "overlay" => "选区窗口显示",
            "ocr" => "文字识别（热）",
            "ocrCold" => "文字识别（冷启动）",
            "ocrLoad" => "模型加载",
            "det" => "检测",
            "rec" => "识别",
            _ => key,
        }
    }
    let s = &stats.system;
    let mut out = String::new();
    let _ = writeln!(out, "Glint 诊断信息");
    let _ = writeln!(
        out,
        "版本: {}  系统: {}  CPU 线程: {}",
        s.app, s.os, s.cpu_threads
    );
    let _ = writeln!(out, "WebView2: {}", s.webview);
    let monitors: Vec<String> = s
        .monitors
        .iter()
        .map(|m| format!("{}×{} @{}%", m.width, m.height, (m.scale * 100.0).round()))
        .collect();
    let _ = writeln!(
        out,
        "显示器: {}",
        if monitors.is_empty() {
            "-".into()
        } else {
            monitors.join(", ")
        }
    );
    let _ = writeln!(out, "统计: 最近 {} 次，自 {}", stats.sessions, stats.since);
    let _ = writeln!(out);
    let _ = writeln!(
        out,
        "{:<16}{:>6}{:>10}{:>10}{:>10}",
        "指标(ms)", "次数", "中位数", "P90", "最大"
    );
    for m in &stats.metrics {
        let _ = writeln!(
            out,
            "{:<16}{:>6}{:>10.0}{:>10.0}{:>10.0}",
            label(m.key),
            m.n,
            m.p50,
            m.p90,
            m.max
        );
    }
    if !stats.engines.is_empty() {
        let _ = writeln!(out);
        let _ = writeln!(
            out,
            "{:<16}{:>6}{:>6}{:>10}{:>10}{:>10}",
            "翻译(ms)", "成功", "失败", "中位数", "P90", "首字"
        );
        for e in &stats.engines {
            let first = e
                .first_token_p50
                .map(|v| format!("{v:.0}"))
                .unwrap_or_else(|| "-".into());
            let note = match e.rate_limited {
                Some(rl) if rl > 0 => format!("  (限流 {rl})"),
                _ => String::new(),
            };
            let _ = writeln!(
                out,
                "{:<16}{:>6}{:>6}{:>10.0}{:>10.0}{:>10}{note}",
                e.engine_name, e.n, e.errors, e.p50, e.p90, first
            );
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn nearest_rank_percentiles() {
        assert_eq!(percentiles(vec![]), None);
        assert_eq!(percentiles(vec![5.0]), Some((5.0, 5.0, 5.0)));
        let v: Vec<f64> = (1..=10).map(f64::from).collect();
        assert_eq!(percentiles(v), Some((5.0, 9.0, 10.0)));
    }

    #[test]
    fn cold_and_warm_ocr_are_separate_metrics() {
        let mut store = Store::default();
        store.samples.push_back(Sample {
            ocr: Some(100.0),
            ..Default::default()
        });
        // Legacy sample without `ocr_wall` falls back to `ocr + ocr_load`.
        store.samples.push_back(Sample {
            ocr: Some(120.0),
            ocr_cold: true,
            ocr_load: Some(300.0),
            ..Default::default()
        });
        let sys = SystemInfo {
            app: "0".into(),
            os: "x".into(),
            cpu_threads: 1,
            webview: "-".into(),
            monitors: vec![],
        };
        let stats = summarize(&store, sys);
        let get = |k| {
            stats
                .metrics
                .iter()
                .find(|m| m.key == k)
                .map(|m| (m.n, m.p50))
        };
        assert_eq!(get("ocr"), Some((1, 100.0)));
        assert_eq!(get("ocrCold"), Some((1, 420.0)));
        assert_eq!(get("ocrLoad"), Some((1, 300.0)));
        assert_eq!(get("capture"), None);
    }

    #[test]
    fn ocr_cold_prefers_actual_post_selection_wall_time() {
        let mut store = Store::default();
        store.samples.push_back(Sample {
            ocr: Some(120.0),
            ocr_wall: Some(150.0), // 300 ms load mostly overlapped with user drag
            ocr_cold: true,
            ocr_load: Some(300.0),
            ..Default::default()
        });
        let sys = SystemInfo {
            app: "0".into(),
            os: "x".into(),
            cpu_threads: 1,
            webview: "-".into(),
            monitors: vec![],
        };
        let stats = summarize(&store, sys);
        let cold = stats.metrics.iter().find(|m| m.key == "ocrCold").unwrap();
        assert_eq!(cold.p50, 150.0);
    }

    #[test]
    fn engine_errors_do_not_count_as_latency() {
        let mut store = Store::default();
        for (ms, error, code) in [
            (Some(200.0), false, None),
            (None, true, Some("rate_limited")),
            (None, true, Some("network")),
            (Some(400.0), false, None),
        ] {
            store.samples.push_back(Sample {
                engines: vec![EngineSample {
                    engine_id: "g".into(),
                    engine_name: "Google".into(),
                    elapsed: ms,
                    first_token: None,
                    error,
                    error_code: code.map(str::to_string),
                }],
                ..Default::default()
            });
        }
        let sys = SystemInfo {
            app: "0".into(),
            os: "x".into(),
            cpu_threads: 1,
            webview: "-".into(),
            monitors: vec![],
        };
        let stats = summarize(&store, sys);
        let e = &stats.engines[0];
        assert_eq!(
            (e.n, e.errors, e.rate_limited, e.p50),
            (2, 2, Some(1), 200.0)
        );
        assert_eq!(e.first_token_p50, None);
        assert!(diagnostics(&stats).contains("(限流 1)"));
    }

    #[test]
    fn timestamps_are_utc_rfc3339() {
        assert_eq!(rfc3339(0), "1970-01-01T00:00:00Z");
        assert_eq!(rfc3339(1_793_254_647), "2026-10-29T06:17:27Z");
        assert_eq!(rfc3339(951_782_400), "2000-02-29T00:00:00Z");
    }
}
