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
                "accent": "#2F6FEB",
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
- Accent default `#2F6FEB`, overridable through `OverlayOptions.accent` (0xRRGGBB).

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

_(none yet)_
