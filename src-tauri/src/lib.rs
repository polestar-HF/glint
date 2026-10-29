//! Glint app shell: tray, hotkeys, config, session state and window management.

mod commands;
mod config;
mod hotkeys;
mod ocr_service;
mod pipeline;
mod popup;
mod session;
mod settings;
mod tray;

use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::{Arc, Mutex, RwLock};
use std::time::Duration;

use tauri::async_runtime::JoinHandle;
use tauri::Manager;
use tauri_plugin_global_shortcut::Shortcut;

use config::Config;
use ocr_service::OcrService;
use pipeline::CaptureMode;
use session::SessionSnapshot;

pub struct AppState {
    config: RwLock<Config>,
    config_path: PathBuf,
    pub ocr: Arc<OcrService>,
    pub http: reqwest::Client,
    pub translate_cache: glint_translate::Cache,
    pub capturing: AtomicBool,
    pub popup_pinned: AtomicBool,
    pub popup_hide_gen: AtomicU64,
    pub popup_anchor: Mutex<Option<popup::Anchor>>,
    pub hotkey_bindings: Mutex<Vec<(Shortcut, CaptureMode)>>,
    session_counter: AtomicU64,
    session: Mutex<Option<SessionSnapshot>>,
    session_task: Mutex<Option<JoinHandle<()>>>,
}

impl AppState {
    pub fn config(&self) -> Config {
        self.config.read().unwrap_or_else(|p| p.into_inner()).clone()
    }

    fn set_config(&self, cfg: Config) {
        *self.config.write().unwrap_or_else(|p| p.into_inner()) = cfg;
    }

    pub fn next_session_id(&self) -> u64 {
        self.session_counter.fetch_add(1, Ordering::SeqCst) + 1
    }

    pub fn set_session(&self, snapshot: SessionSnapshot) {
        *self.session.lock().unwrap_or_else(|p| p.into_inner()) = Some(snapshot);
    }

    pub fn session_snapshot(&self) -> Option<SessionSnapshot> {
        self.session.lock().unwrap_or_else(|p| p.into_inner()).clone()
    }

    /// Apply `f` only if `session_id` is still the current session.
    pub fn update_session(&self, session_id: u64, f: impl FnOnce(&mut SessionSnapshot)) {
        let mut guard = self.session.lock().unwrap_or_else(|p| p.into_inner());
        if let Some(s) = guard.as_mut().filter(|s| s.session_id == session_id) {
            f(s);
        }
    }

    /// A new session supersedes the old one: abort its OCR/translation work.
    pub fn replace_session_task(&self, task: JoinHandle<()>) {
        if let Some(old) = self.session_task.lock().unwrap_or_else(|p| p.into_inner()).replace(task)
        {
            old.abort();
        }
    }
}

pub fn run() {
    tauri::Builder::default()
        .plugin(tauri_plugin_single_instance::init(|app, _args, _cwd| settings::open(app)))
        .plugin(
            tauri_plugin_global_shortcut::Builder::new()
                .with_handler(|app, shortcut, event| hotkeys::handle(app, shortcut, event))
                .build(),
        )
        .plugin(tauri_plugin_autostart::init(
            tauri_plugin_autostart::MacosLauncher::LaunchAgent,
            Some(vec!["--background"]),
        ))
        .plugin(tauri_plugin_clipboard_manager::init())
        .plugin(
            tauri_plugin_log::Builder::new()
                .level(log::LevelFilter::Info)
                .max_file_size(1_000_000)
                .build(),
        )
        .setup(|app| {
            let handle = app.handle().clone();
            let config_dir = app.path().app_config_dir()?;
            let config_path = config::config_path(&config_dir);
            let cfg = config::load(&config_path);
            let resource_dir = app.path().resource_dir()?;
            let model_dir = resource_dir.join("models").join("v6-small");
            let ort_dylib = resource_dir.join("onnxruntime.dll");

            let http = glint_translate::http_client();

            app.manage(AppState {
                config: RwLock::new(cfg.clone()),
                config_path,
                ocr: Arc::new(OcrService::new(model_dir, ort_dylib)),
                http,
                translate_cache: glint_translate::Cache::default(),
                capturing: AtomicBool::new(false),
                popup_pinned: AtomicBool::new(false),
                popup_hide_gen: AtomicU64::new(0),
                popup_anchor: Mutex::new(None),
                hotkey_bindings: Mutex::new(Vec::new()),
                session_counter: AtomicU64::new(0),
                session: Mutex::new(None),
                session_task: Mutex::new(None),
            });

            tray::create(&handle)?;
            if let Err(e) = hotkeys::register(&handle, &cfg.hotkeys) {
                log::warn!("{e:#}");
            }
            spawn_idle_unloader(handle.clone());

            // First launch (no config yet) opens settings so the user sees the app exists.
            let first_run = !app.state::<AppState>().config_path.exists();
            let background = std::env::args().any(|a| a == "--background");
            if first_run && !background {
                settings::open(&handle);
            }
            Ok(())
        })
        .invoke_handler(tauri::generate_handler![
            commands::get_config,
            commands::save_config,
            commands::get_session_snapshot,
            commands::retranslate,
            commands::test_engine,
            commands::copy_text,
            commands::popup_set_pinned,
            commands::popup_hide,
            commands::popup_resize,
            commands::start_capture,
            commands::open_settings,
        ])
        .build(tauri::generate_context!())
        .expect("failed to build Glint")
        .run(|_app, event| {
            // Tray app: closing the last window must not quit.
            if let tauri::RunEvent::ExitRequested { api, code: None, .. } = event {
                api.prevent_exit();
            }
        });
}

fn spawn_idle_unloader(app: tauri::AppHandle) {
    tauri::async_runtime::spawn(async move {
        let mut tick = tokio::time::interval(Duration::from_secs(30));
        loop {
            tick.tick().await;
            let state = app.state::<AppState>();
            let secs = state.config().ocr.idle_unload_secs;
            if secs > 0 {
                state.ocr.unload_if_idle(Duration::from_secs(secs));
            }
        }
    });
}
