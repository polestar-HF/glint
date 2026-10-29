//! Capture → select → OCR → translate.

use std::sync::atomic::Ordering;
use std::sync::mpsc::Sender;
use std::time::{Duration, Instant};

use glint_overlay::{Action, Outcome, OverlayOptions, Selection};
use glint_translate::{EngineEvent, TranslateRequest};
use tauri::{AppHandle, Emitter, Manager};
use tauri_plugin_clipboard_manager::ClipboardExt;

use crate::popup::{self, Anchor};
use crate::session::*;
use crate::AppState;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CaptureMode {
    Translate,
    Ocr,
    Silent,
}

pub struct OverlayJob {
    opts: OverlayOptions,
    popup_was_visible: bool,
}

/// The overlay pumps its own Win32 message loop, so it runs on one long-lived thread of its
/// own (never the Tauri main thread or a tokio worker); jobs arrive over a channel.
pub fn spawn_overlay_worker(app: AppHandle) -> std::io::Result<Sender<OverlayJob>> {
    let (tx, rx) = std::sync::mpsc::channel::<OverlayJob>();
    std::thread::Builder::new()
        .name("glint-overlay".into())
        .spawn(move || {
            for job in rx {
                if job.popup_was_visible {
                    // Give DWM a frame to actually remove the popup before BitBlt.
                    std::thread::sleep(Duration::from_millis(40));
                }
                let outcome = glint_overlay::capture_all()
                    .and_then(|frames| glint_overlay::select_region(frames, &job.opts));
                app.state::<AppState>()
                    .capturing
                    .store(false, Ordering::SeqCst);
                match outcome {
                    Ok(Outcome::Selected(sel)) => run_session(&app, sel),
                    Ok(Outcome::Cancelled) => {}
                    Err(e) => log::error!("capture failed: {e}"),
                }
            }
        })?;
    Ok(tx)
}

/// Entry point for hotkeys, tray and the `start_capture` command.
pub fn start_capture(app: &AppHandle, mode: CaptureMode) {
    let state = app.state::<AppState>();
    if state.capturing.swap(true, Ordering::SeqCst) {
        return; // overlay already up: ignore the repeated hotkey
    }
    let cfg = state.config();
    state.ocr.preload(cfg.ocr.engine.clone());

    // Keep our own popup out of the frozen frame.
    let popup_was_visible = popup::is_visible(app);
    if popup_was_visible {
        popup::hide(app);
    }

    let opts = OverlayOptions {
        default_action: match mode {
            CaptureMode::Translate => Action::Translate,
            CaptureMode::Ocr => Action::OcrOnly,
            CaptureMode::Silent => Action::CopyText,
        },
        accent: cfg.ui.accent_rgb(),
        snap_windows: cfg.overlay.snap_windows,
        show_magnifier: cfg.overlay.magnifier,
    };
    if state
        .overlay_tx
        .send(OverlayJob {
            opts,
            popup_was_visible,
        })
        .is_err()
    {
        log::error!("overlay worker is gone");
        state.capturing.store(false, Ordering::SeqCst);
    }
}

fn run_session(app: &AppHandle, sel: Selection) {
    let state = app.state::<AppState>();
    let cfg = state.config();
    let session_id = state.next_session_id();
    let silent = sel.action == Action::CopyText;
    let mode = if sel.action == Action::OcrOnly {
        Mode::Ocr
    } else {
        Mode::Translate
    };

    if !silent {
        begin_session(
            app,
            session_id,
            mode,
            &cfg.translate.source_lang,
            &cfg.translate.target_lang,
        );
        popup::show_at(
            app,
            Anchor {
                rect: sel.rect,
                work_area: sel.work_area,
                scale: sel.scale as f64,
            },
        );
    }

    let app = app.clone();
    let task = tauri::async_runtime::spawn(async move {
        let state = app.state::<AppState>();
        let ocr = state.ocr.clone();
        let engine_kind = cfg.ocr.engine.clone();
        let opts = glint_ocr::OcrOptions {
            merge_lines: cfg.ocr.merge_lines,
            lang_hint: cfg.translate.source_lang.clone(),
        };
        let started = Instant::now();
        let result = tokio::task::spawn_blocking(move || {
            let engine = ocr.get(&engine_kind)?;
            engine
                .recognize(&sel.image, &opts)
                .map_err(anyhow::Error::from)
        })
        .await
        .map_err(anyhow::Error::from)
        .and_then(|r| r);
        let elapsed_ms = started.elapsed().as_millis() as u64;

        let result = match result {
            Ok(r) => r,
            Err(e) => {
                log::error!("ocr failed: {e:#}");
                let message = ocr_user_message(&e);
                if !silent {
                    state.update_session(session_id, |s| {
                        s.ocr.status = Status::Error;
                        s.ocr.message = Some(message.clone());
                    });
                    let _ = app.emit(
                        "ocr:error",
                        SessionError {
                            session_id,
                            message,
                        },
                    );
                }
                return;
            }
        };

        if silent {
            if !result.text.is_empty() {
                if let Err(e) = app.clipboard().write_text(result.text.clone()) {
                    log::error!("clipboard write failed: {e}");
                }
            }
            return;
        }

        let payload = OcrPayload {
            session_id,
            result: result.clone(),
            elapsed_ms,
        };
        state.update_session(session_id, |s| {
            s.ocr.status = Status::Done;
            s.ocr.result = Some(payload.clone());
        });
        let _ = app.emit("ocr:result", payload);

        if mode == Mode::Translate && !result.text.trim().is_empty() {
            let target = effective_target(
                &cfg.translate.source_lang,
                result.detected_lang.as_deref(),
                &cfg.translate.target_lang,
                &cfg.translate.secondary_lang,
            );
            if target != cfg.translate.target_lang {
                state.update_session(session_id, |s| s.target_lang = target.clone());
                // The popup learned the configured target from `session:start`; tell it which
                // language is really used (a repeated start for the same id only updates it).
                let _ = app.emit(
                    "session:start",
                    SessionStart {
                        session_id,
                        mode,
                        source_lang: cfg.translate.source_lang.clone(),
                        target_lang: target.clone(),
                    },
                );
            }
            translate(
                &app,
                session_id,
                result.text,
                cfg.translate.source_lang.clone(),
                target,
            )
            .await;
        }
    });
    state.replace_session_task(task);
}

/// Start a translation-only session (popup edits, language switch).
pub fn retranslate(app: &AppHandle, text: String, source: String, target: String) -> u64 {
    let state = app.state::<AppState>();
    let session_id = state.next_session_id();
    begin_session(app, session_id, Mode::Translate, &source, &target);
    state.update_session(session_id, |s| {
        s.ocr.status = Status::Done;
    });
    let app2 = app.clone();
    let task = tauri::async_runtime::spawn(async move {
        translate(&app2, session_id, text, source, target).await;
    });
    state.replace_session_task(task);
    session_id
}

fn begin_session(app: &AppHandle, session_id: u64, mode: Mode, source: &str, target: &str) {
    let state = app.state::<AppState>();
    state.set_session(SessionSnapshot::new(
        session_id,
        mode,
        source.into(),
        target.into(),
    ));
    let _ = app.emit(
        "session:start",
        SessionStart {
            session_id,
            mode,
            source_lang: source.into(),
            target_lang: target.into(),
        },
    );
}

/// Re-run one engine inside the current session (the popup's per-engine "重试").
pub fn retranslate_engine(
    app: &AppHandle,
    session_id: u64,
    engine_id: String,
    text: String,
    source: String,
    target: String,
) {
    let state = app.state::<AppState>();
    if !state.is_current_session(session_id) {
        return;
    }
    let app = app.clone();
    // Not tracked as the session task: a retry must not abort the other engines.
    tauri::async_runtime::spawn(async move {
        translate_with(&app, session_id, text, source, target, Some(&engine_id)).await;
    });
}

async fn translate(app: &AppHandle, session_id: u64, text: String, source: String, target: String) {
    translate_with(app, session_id, text, source, target, None).await;
}

async fn translate_with(
    app: &AppHandle,
    session_id: u64,
    text: String,
    source: String,
    target: String,
    only_engine: Option<&str>,
) {
    let state = app.state::<AppState>();
    let cfg = state.config();
    let mut engines = Vec::new();
    let selected = cfg.translate.engines.iter().filter(|e| match only_engine {
        Some(id) => e.id == id,
        None => e.enabled,
    });
    for ec in selected {
        match glint_translate::build_cached(ec, &state.http, &state.translate_cache) {
            Ok(engine) => engines.push(engine),
            Err(e) => {
                let name = ec
                    .name
                    .clone()
                    .unwrap_or_else(|| glint_translate::default_name(&ec.kind).to_string());
                emit_tr_error(app, session_id, &ec.id, &name, e.user_message());
            }
        }
    }
    if engines.is_empty() {
        return;
    }
    let req = TranslateRequest {
        text,
        source_lang: source,
        target_lang: target,
        secondary_lang: Some(cfg.translate.secondary_lang.clone()),
    };
    let app2 = app.clone();
    glint_translate::translate_all(&engines, &req, move |ev| {
        on_engine_event(&app2, session_id, ev)
    })
    .await;
}

fn on_engine_event(app: &AppHandle, session_id: u64, ev: EngineEvent) {
    let state = app.state::<AppState>();
    match ev {
        EngineEvent::Start {
            engine_id,
            engine_name,
        } => {
            state.update_session(session_id, |s| {
                let fresh = TranslationState {
                    engine_id: engine_id.clone(),
                    engine_name: engine_name.clone(),
                    status: Status::Pending,
                    text: String::new(),
                    delta_count: 0,
                    message: None,
                    elapsed_ms: None,
                };
                // A per-engine retry restarts an existing entry in place.
                match s.translation_mut(&engine_id) {
                    Some(t) => *t = fresh,
                    None => s.translations.push(fresh),
                }
            });
            let _ = app.emit(
                "tr:start",
                TrStart {
                    session_id,
                    engine_id,
                    engine_name,
                },
            );
        }
        EngineEvent::Delta { engine_id, delta } => {
            let seq = state
                .update_session(session_id, |s| {
                    s.translation_mut(&engine_id).map(|t| {
                        t.status = Status::Streaming;
                        t.text.push_str(&delta);
                        t.delta_count += 1;
                        t.delta_count - 1
                    })
                })
                .flatten();
            // A stale session has no snapshot entry; the popup ignores its events anyway.
            let Some(seq) = seq else { return };
            let _ = app.emit(
                "tr:delta",
                TrDelta {
                    session_id,
                    engine_id,
                    delta,
                    seq,
                },
            );
        }
        EngineEvent::Done {
            engine_id,
            text,
            elapsed_ms,
        } => {
            state.update_session(session_id, |s| {
                if let Some(t) = s.translation_mut(&engine_id) {
                    t.status = Status::Done;
                    t.text = text.clone();
                    t.elapsed_ms = Some(elapsed_ms);
                }
            });
            let _ = app.emit(
                "tr:done",
                TrDone {
                    session_id,
                    engine_id,
                    text,
                    elapsed_ms,
                },
            );
        }
        EngineEvent::Error { engine_id, message } => {
            state.update_session(session_id, |s| {
                if let Some(t) = s.translation_mut(&engine_id) {
                    t.status = Status::Error;
                    t.message = Some(message.clone());
                }
            });
            let _ = app.emit(
                "tr:error",
                TrError {
                    session_id,
                    engine_id,
                    message,
                },
            );
        }
    }
}

fn emit_tr_error(app: &AppHandle, session_id: u64, engine_id: &str, name: &str, message: String) {
    on_engine_event(
        app,
        session_id,
        EngineEvent::Start {
            engine_id: engine_id.into(),
            engine_name: name.into(),
        },
    );
    on_engine_event(
        app,
        session_id,
        EngineEvent::Error {
            engine_id: engine_id.into(),
            message,
        },
    );
}

/// One short line for the popup; details stay in the log.
fn ocr_user_message(e: &anyhow::Error) -> String {
    match e.downcast_ref::<glint_ocr::OcrError>() {
        Some(glint_ocr::OcrError::Load(_)) => "识别模型加载失败，请尝试重新安装 Glint".into(),
        Some(glint_ocr::OcrError::Unsupported) => "当前识别引擎不可用，请在设置中切换".into(),
        _ => "文字识别失败".into(),
    }
}

/// If the text is already in the target language, translate into the secondary language
/// instead (e.g. Chinese text with target zh-Hans → English). With source "auto" that is
/// decided by the language OCR detected; an explicitly chosen source is the user's word for
/// what the text is, so a mismatching guess must not override the chosen pair.
pub fn effective_target(
    source: &str,
    detected: Option<&str>,
    target: &str,
    secondary: &str,
) -> String {
    let primary = |code: &str| code.split('-').next().unwrap_or(code).to_ascii_lowercase();
    if secondary.trim().is_empty() || glint_translate::langs::is_auto(secondary) {
        return target.to_string();
    }
    let already_target = if glint_translate::langs::is_auto(source) {
        detected.is_some_and(|d| primary(d) == primary(target))
    } else {
        glint_translate::langs::same_lang(source, target)
    };
    if already_target {
        secondary.to_string()
    } else {
        target.to_string()
    }
}

#[cfg(test)]
mod tests {
    use super::effective_target;

    #[test]
    fn flips_to_secondary_when_source_is_target() {
        let auto = |d, t, s| effective_target("auto", d, t, s);
        assert_eq!(auto(Some("zh"), "zh-Hans", "en"), "en");
        assert_eq!(auto(Some("en"), "zh-Hans", "en"), "zh-Hans");
        assert_eq!(auto(None, "zh-Hans", "en"), "zh-Hans");
        assert_eq!(auto(Some("en"), "en", "zh-Hans"), "zh-Hans");
        assert_eq!(auto(Some("zh"), "zh-Hans", ""), "zh-Hans");
    }

    #[test]
    fn explicit_source_is_not_overridden_by_detection() {
        // Kanji-only Japanese is detected as "zh", but the user said it is Japanese.
        assert_eq!(
            effective_target("ja", Some("zh"), "zh-Hans", "en"),
            "zh-Hans"
        );
        assert_eq!(effective_target("zh-Hans", None, "zh-Hans", "en"), "en");
        assert_eq!(
            effective_target("zh-Hant", None, "zh-Hans", "en"),
            "zh-Hans"
        );
    }
}
