# Glint — Architecture & Team Contract

Glint is a Windows screenshot OCR and translation tool. You press a hotkey, the screen
freezes, you drag a region (or click a window), and a small popup shows the recognized
text and its translation.

Product priorities, in order:
1. **Responsive.** The overlay shows within 50 ms of the hotkey. OCR results appear in under 300 ms for typical regions.
2. **Light.** Idle in the tray (popup destroyed, OCR unloaded) stays under 40 MB, summed over all processes. Active use stays under 250 MB. The installer stays under 45 MB, OCR models included.
3. **Accurate.** OCR and translation should be as accurate as the engines allow.
4. **Refined UI.** Calm and typographic, with nothing that looks templated or "AI-generated".

## Repository layout and ownership

| Path | Owner | Notes |
|---|---|---|
| `crates/glint-overlay` | capture engineer | Pure Win32 + Direct2D. Captures the screen and runs the fullscreen selection overlay. No Tauri dependency. |
| `crates/glint-ocr` | OCR engineer | PP-OCR (ONNX Runtime) plus Windows.Media.Ocr. Also handles pre- and post-processing. No Tauri dependency. |
| `crates/glint-translate` | translation engineer | Async translation engines with streaming. No Tauri dependency. |
| `src-tauri` | lead | App shell: hotkeys, tray, config, wiring, IPC commands and events. |
| `ui` | UI engineer | Svelte 5 + Vite + TS. Two pages: `popup.html` and `settings.html`. Also owns `docs/DESIGN.md`. |
| `.github/workflows` | CI engineer | Windows build, NSIS installer, portable zip. |

Do not edit files outside your area. If you need a contract change, write it under
"Contract change requests" at the end of this file and tell the lead.

## Process model and memory budget

- **Main process (Rust).** Owns the tray, global hotkeys, the overlay thread and the OCR
  engine. The OCR engine loads lazily on first use, then unloads after `ocr.idle_unload_secs`
  of inactivity (default 300 s).
- **Selection overlay: native, not a webview.** One borderless topmost window per monitor.
  It is created at hotkey time and drawn with Direct2D over the frozen frame, so it shows
  instantly, gives a true freeze frame and costs no extra process.
- **Popup: one WebView2 window.** It is created lazily on the first capture, in parallel
  with OCR, so its cold start overlaps recognition. It is reused while in use and destroyed
  after `ui.popupIdleDestroySecs` hidden (default 600 s; 0 keeps it resident). The popup page
  must render a useful "recognizing" state within one frame of loading. It must also tolerate
  events that arrived before it subscribed: on mount it calls `get_session_snapshot` to catch up.
- **Settings: a WebView2 window created on demand** and destroyed when closed.
- **OCR default model: PP-OCRv6 small** (det 9.9 MB + rec 21 MB; covers zh, en, ja and 47
  more). ONNX Runtime sessions use 2–4 intra-op threads and no memory arena, to limit RAM.

## Flow

```
hotkey ──► glint_overlay::capture_all()          (all monitors, physical px, BGRA)
       ──► glint_overlay::select_region(frames)   (blocking, on the overlay thread)
            └─ Outcome::Selected(Selection{ image: RgbaImage, rect, scale, work_area, action })
       ──► show popup near rect (state "recognizing")   emit `session:start`
       ──► glint_ocr  engine.recognize(&image)          emit `ocr:result` | `ocr:error`
       ──► for each enabled engine, concurrently:
             glint_translate  translate(req, delta_tx)  emit `tr:start`, `tr:delta`*, `tr:done` | `tr:error`
```

`action` comes from the overlay. **Enter** or mouse-up means `Translate` (or `OcrOnly`
when the overlay was opened by the OCR hotkey). **Ctrl+C** means `CopyText`: run OCR,
put the text on the clipboard, show a small toast and skip the popup.

## Crate contracts

The authoritative signatures live in each crate's `src/lib.rs` skeleton. You may add
items, but do not rename or remove contract items without a change request.

### glint-overlay
- `capture_all() -> Result<Vec<MonitorFrame>>`. Covers every monitor, in physical pixels, and is per-monitor DPI aware.
- `select_region(frames: Vec<MonitorFrame>, opts: &OverlayOptions) -> Result<Outcome>`. This call blocks and runs its own message loop on the calling thread.
- On non-Windows targets, both functions return `Err(Unsupported)`, so the workspace still builds on Linux.

### glint-ocr
- `trait OcrEngine { fn recognize(&self, img: &RgbaImage, opts: &OcrOptions) -> Result<OcrResult> }`
- `PaddleOcr::load(model_dir)`. Cross-platform, so it can be tested on Linux.
- `WindowsOcr::new(lang)`. Windows only.
- `OcrResult` holds the raw lines with boxes and confidence, plus `text`: paragraphs
  reconstructed from those lines. This is the text that gets translated.

### glint-translate
- `trait Translator { fn id(); fn name(); async fn translate(&self, req: &TranslateRequest, deltas: DeltaSink) -> Result<String> }`
- `build(engine_cfg: &EngineConfig, http: &reqwest::Client) -> Result<Box<dyn Translator>>`
- An engine with no streaming support sends one delta containing the full text.

## IPC (popup / settings ↔ Rust)

Event payloads are camelCase JSON. Every event carries `sessionId: u64`. The UI ignores
events whose session id is not the latest.

| Event | Payload |
|---|---|
| `session:start` | `{ sessionId, mode: "translate"\|"ocr", sourceLang, targetLang }` |
| `ocr:result` | `{ sessionId, text, lines: [{text, box:[x,y,w,h], confidence}], detectedLang, elapsedMs, engine }` |
| `ocr:error` | `{ sessionId, message }` |
| `tr:start` | `{ sessionId, engineId, engineName }` |
| `tr:delta` | `{ sessionId, engineId, delta }` |
| `tr:done` | `{ sessionId, engineId, text, elapsedMs }` |
| `tr:error` | `{ sessionId, engineId, message }` |
| `config:changed` | the full `Config` |

`get_session_snapshot() -> SessionSnapshot | null` returns `{ sessionId, mode, sourceLang,
targetLang, ocr: {status: "pending"|"done"|"error", result?, message?}, translations:
[{engineId, engineName, status: "pending"|"streaming"|"done"|"error", text, message?,
elapsedMs?}] }`. A freshly created popup uses it to catch up.

| Command | Signature |
|---|---|
| `get_config` | `() -> Config` |
| `save_config` | `(config: Config) -> ()` |
| `retranslate` | `(text: String, sourceLang: String, targetLang: String) -> u64` (new sessionId; the OCR step is skipped) |
| `test_engine` | `(engine: EngineConfig) -> String` (translates "Hello, world." as a smoke test) |
| `copy_text` | `(text: String) -> ()` |
| `popup_set_pinned` | `(pinned: bool) -> ()`. While pinned, the popup does not hide on blur. |
| `popup_hide` | `() -> ()` |
| `popup_resize` | `(height: f64) -> ()` (logical px; the UI reports its content height) |
| `start_capture` | `(mode: "translate"\|"ocr") -> ()` |
| `open_settings` | `() -> ()` |

## Config

The config file is `%APPDATA%\Glint\config.json`. The schema is implemented in
`src-tauri/src/config.rs`. All keys are camelCase, and missing keys fall back to their
defaults. API keys live in the same file under each engine's `apiKey` field. Encrypting
them with DPAPI is planned for later.

```jsonc
{
  "general":  { "uiLanguage": "zh-CN",            // "zh-CN" | "en-US"
                "autostart": false },
  "hotkeys":  { "translate": "Alt+Q",             // screenshot → OCR → translate popup
                "ocr": "Alt+W",                   // screenshot → OCR popup (no translation)
                "silentOcr": "Alt+Shift+W" },     // screenshot → OCR → clipboard + toast
  "ocr":      { "engine": "paddle",               // "paddle" | "windows"
                "mergeLines": true,
                "idleUnloadSecs": 300 },
  "translate":{ "sourceLang": "auto",
                "targetLang": "zh-Hans",
                "secondaryLang": "en",            // used when detected source == target
                "engines": [ /* EngineConfig, display order */ ] },
  "ui":       { "theme": "system",                // "system" | "light" | "dark"
                "fontSize": 14,
                "accent": "#1E6AC5",
                "closeOnBlur": true,
                "popupIdleDestroySecs": 600 },
  "overlay":  { "snapWindows": true, "magnifier": true }
}
```

Language codes are BCP-47: `auto zh-Hans zh-Hant en ja ko fr de es ru pt it vi th ar`.
`glint-translate` maps them to each engine's own codes.

The default engine list is `google` (enabled), `bing` (enabled), then `anthropic` (id
`claude`, model `claude-haiku-5-5`), `openai`, `deepl`, `youdao` and `baidu`, all disabled.
The `EngineConfig` fields are defined in `glint-translate/src/lib.rs`.

## Overlay visual spec (initial; the UI engineer may refine it in DESIGN.md)

- Dim layer: black at 45 % opacity over the frozen frame. The selection itself is not dimmed.
- Selection border: 1 physical px × scale, in the accent color. Eight 6×6 logical px square handles, white with a 1 px accent border.
- Size chip: shown above the top-left corner (below it if there is no room). Text `1280 × 720` in Segoe UI Variable / Segoe UI at 12 px. Background `#1F1F1F` at 90 % opacity, text `#F2F2F2`, 4 px corner radius, 4×8 px padding.
- Window snap: before dragging starts, the window under the cursor is outlined with a 2 px accent border, with no fill.
- Magnifier: shown during drag and before dragging starts. A 120×120 logical px box at 8× zoom, pixel grid, crosshair, and a hex color line under it. It sits at the bottom-right of the cursor and flips at screen edges.
- Cursor: crosshair.
- No animations, which keeps the overlay instant.
- Accent default `#1E6AC5` (see DESIGN.md §3), overridable through `OverlayOptions.accent` (0xRRGGBB).

## Conventions

- Rust edition 2021, MSRV is whatever is installed (1.97). Use `thiserror` in libraries and `anyhow` only in `src-tauri`.
- Never `unwrap()` on I/O, FFI or network paths. Log with the `log` crate.
- Only the `windows` crate is used for Win32/WinRT (no `winapi`), on version `0.61`.
- Build environment: a Linux cloud container with restricted egress.
  - Reachable: crates.io, npm and PyPI.
  - Blocked: Microsoft download hosts, HuggingFace, cdn.pyke.io and GitHub release assets.
    That means `cargo xwin` cannot fetch the MSVC CRT.
- Build checks that work here:
  - Windows type-check, no linking: `cargo check --target x86_64-pc-windows-msvc -p <crate>`.
  - Linux: `cargo check -p <crate>` and `cargo test -p <crate>`. `src-tauri` needs webkit2gtk, so it is checked only for the Windows target.
  - Use your own target dir to avoid lock contention between agents: `CARGO_TARGET_DIR=/home/user/glint/target/<your-area>`.
  - Real linking and the installer come from GitHub Actions (windows-latest).
- Test assets:
  - The PP-OCRv6 small ONNX models are inside the `rapidocr` PyPI wheel, at `rapidocr/models/PP-OCRv6_{det,rec}_small.onnx`.
  - A Linux ONNX Runtime 1.31 shared library can be obtained from PyPI or npm packages.
  - Never commit models or binaries (see `.gitignore`).
- Keep dependencies lean and justify each new crate in your PR notes.

## Contract change requests

### glint-translate (translation engineer)

1. **`TranslateRequest` gains `secondary_lang: Option<String>`** (asked for by the lead). Code that builds the struct with a literal must add the field, or use `TranslateRequest::new(text, source, target)` or `..Default::default()`. The app should fill it from `translate.secondaryLang`. With source `auto`, or a source equal to the target, the engines translate into this language when the text is already in the target language.
2. Everything else is additive: `translate_all` and `EngineEvent`, `Cache` and `build_cached`, `http_client()` (use it so the 5 s connect timeout applies), `default_engines()`, `langs`, and `TranslateError::user_message()` (a short UI-ready string; `Display` keeps its technical prefix). The `Translator` trait and `build` are unchanged.

### glint-overlay (capture engineer)

1. **Root `Cargo.toml` (lead): add `[profile.dev.package.glint-overlay] opt-level = 3`.** `[profile.dev.package."*"]` does not cover workspace members. Without it, debug builds run the per-pixel capture copy (alpha forced to 255) and the overlay scene unoptimized, which adds tens of ms on a 4K screen. Release builds are unaffected. There is no API change.
2. Behaviour notes, with no signature change:
   - `select_region` returns `OverlayError::Window` if an overlay is already open in the process.
   - A `WM_QUIT` that arrives while the overlay is open cancels it, and is re-posted for the caller's loop.
   - The snapped window is shown undimmed, like a selection, with the 2 px accent outline and no tint. The UI engineer may want to confirm this in DESIGN.md.

### ui (UI engineer)

Items 1, 3, 4 and 5 are optional for the UI: it detects a missing command or field and falls back, as noted in each item.

1. **New command `retranslate_engine(sessionId: u64, engineId: String, text: String, sourceLang: String, targetLang: String) -> ()`.** This is the per-engine "重试" on an error line. It re-runs one engine inside the *current* session and emits `tr:start`, then `tr:delta`*, then `tr:done` or `tr:error`, all with the same `sessionId`. Fallback: if the command is missing, the UI calls `retranslate`.
2. **Clarification for `retranslate`.** Emit `session:start { sessionId: <new>, mode: "translate", sourceLang, targetLang }` before any `tr:*` event for the new id, and emit no `ocr:*` events. The popup keeps the edited text, and it copes if the event arrives before the command resolves.
3. **`tr:delta` gains `seq: u32`.** This is a 0-based index per engine per session. Snapshot translations gain `deltaCount: u32`, the number of deltas already folded into `text`. Both are additive. They let the popup drop deltas that are replayed around `get_session_snapshot`, instead of guessing. Fallback: without them, the UI skips a replayed delta that the snapshot text already ends with, and `tr:done` reconciles.
4. **New command `check_hotkey(accelerator: String) -> "ok" | "in_use" | "invalid"`.** It test-registers and immediately unregisters the shortcut, without saving. The settings key recorder calls it before committing a new shortcut, to catch shortcuts taken by other apps (QQ and WeChat use Ctrl+Alt+A, for example). Fallback: if the command is missing, the UI trusts its local validation.
5. **New command `hotkeys_suspend(suspended: bool) -> ()`.** It unregisters Glint's global shortcuts while the key recorder is listening, and re-registers them afterwards. Without it, pressing the current shortcut in the recorder starts a capture. Resume automatically if the settings window closes while suspended.
6. **Accelerator string format.** The UI writes `Ctrl+Alt+Shift+Super+<Key>`: modifiers in that order, then exactly one key named after `KeyboardEvent.code` without the `Key`, `Digit` or `Arrow` prefix. Keys are:
   - `A`–`Z`, `0`–`9`, `F1`–`F24`
   - `Space`, `PrintScreen`, `Pause`, `Insert`, `Delete`, `Home`, `End`, `PageUp`, `PageDown`
   - `Up`, `Down`, `Left`, `Right`
   - `Backquote`, `Minus`, `Equal`, `BracketLeft`, `BracketRight`, `Backslash`, `Semicolon`, `Quote`, `Comma`, `Period`, `Slash`
   - `Numpad0`–`Numpad9`, `NumpadAdd`, and so on

   The UI displays `Super` as "Win". An empty string means the shortcut is disabled. This is the syntax that `tauri-plugin-global-shortcut` / `global-hotkey` parses.
7. **Default accent becomes `#1E6AC5`.** Change the `ui.accent` default in `config.rs`, and `OverlayOptions::default().accent` to `0x1E6AC5`. The rationale is in DESIGN.md §3. The overlay keeps using the stored value as is; the UI derives the dark-theme variant itself.
8. **Value semantics.** `ocr.idleUnloadSecs = 0` and `ui.popupIdleDestroySecs = 0` mean "never". The UI offers 60 / 300 / 900 / 0 for the first and 60 / 600 / 1800 / 0 for the second.
9. **`tr:error.message` and `ocr:error.message` should carry the short `TranslateError::user_message()`-style text.** The popup shows one ellipsized line, with the full text in a tooltip.
10. **The popup never writes config.** Language changes in the popup apply to the current session only, through `retranslate`.
11. **Window and CSP settings for `tauri.conf.json` and capabilities.** These are listed in DESIGN.md §12. They include popup `shadow: true` and `maximizable: false`, the Mica effect with `popup.html?mica=0` on Windows 10, `core:window:allow-set-theme`, and `tauri-plugin-opener` with `opener:allow-open-url` for the About links.

### glint-ocr (OCR engineer)

1. **`PaddleOcr::load(model_dir: &Path, opts: PaddleOptions) -> Result<Self>`** replaces `load(model_dir)`. `PaddleOptions::default()` is right for the app except `ort_dylib_path`, which the app must set to the absolute path of the shipped `onnxruntime.dll` (`<resource_dir>/onnxruntime.dll`); Windows 11 ships an older `onnxruntime.dll` in `System32`, and a path that is given but missing is an error, never a fallback. The DLL must be ONNX Runtime 1.31 or newer. ONNX Runtime is initialized once per process (the first load wins, including its path); load, drop (idle unload) and load again are safe and reuse it.
2. **Cargo features of `glint-ocr`:** `default = ["ort-dynamic"]` (runtime loading, no build-time download) and opt-in `ort-static` (`ort`'s `download-binaries`; on Windows that is the DirectML build, which also adds load-time imports of `DirectML.dll`/`d3d12.dll`/`dxgi.dll`). Do not enable both. With neither, `PaddleOcr::load` returns `OcrError::Unsupported`.
3. **Model directory:** `det.onnx` and `rec.onnx` (PP-OCRv6 small from the `rapidocr==3.10.0` wheel, renamed), optional `cls.onnx` (only read with `PaddleOptions::use_angle_cls`), optional `dict.txt` (only used if `rec.onnx` has no embedded `character` metadata; the v6 model has one).
4. **Result semantics:** `OcrResult.lines` are in reading order with normalized text; `text` joins paragraphs (or, with `merge_lines = false`, lines) with a single `\n`; `detected_lang` is one of `zh ja ko en fr de es pt it vi ru th ar`, or `None` when the text has no letters; `engine` is `"paddle"` or `"windows"`. `OcrOptions::lang_hint` is ignored by `PaddleOcr` (the multilingual model needs none) and selects the language pack for `WindowsOcr`. `WindowsOcr` reports `confidence = 1.0` (the API gives none) and must be called from a worker thread, not an STA/UI thread.
5. **Additive items:** `PaddleOptions`, `Timings` and `PaddleOcr::recognize_timed`, `WindowsOcr::{available_languages, language}`, the `postprocess` module, and `glint_ocr::trim_process_memory()`. Call the last one after dropping the engine on idle unload: glibc and Windows keep freed pages in the process until trimmed (measured on Linux: 85 MB resident after drop, 40 MB after trimming).
