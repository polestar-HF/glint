# Research notes (2026-10-29)

Condensed from the researcher's report. Items marked (U) are unverified estimates, so measure them on real hardware.

## Comparable tools: what to copy, what to avoid

- **Pot** (Tauri 1, archived Sep 2026).
  - Copy: running engines in parallel, one card per engine.
  - Avoid:
    - Its pipeline is slow: it writes a PNG to disk, loads it in a webview, crops it, re-encodes a PNG, and only then runs OCR.
    - It does no preprocessing, so small regions return no text (#903).
    - It breaks across monitors with different DPI (#1161), because it computes coordinates with `naturalWidth/screen.width`.
- **STranslate** (WPF).
  - Copy: offline PaddleOCR, silent OCR, the image-translation overlay, and quick service switching.
  - Avoid: it needed repeated memory-leak fixes.
- **Umi-OCR**.
  - Copy: its 8 text-block post-processors, reading order via a gap-tree, and paragraph-end prediction.
  - Copy: "engine memory cleanup" after idle.
- **Text Grab / PowerToys Text Extractor**.
  - Copy: the freeze-frame overlay (a `CopyFromScreen` bitmap), one overlay window per screen, and the text-cleanup editor.
- **Snipaste**.
  - Copy: its `squeeze_memory` trade-off.
- **Bob / Easydict** (macOS).
  - Copy: silent OCR to clipboard, stacked services, and editable source text with a re-translate action.

## Patterns we adopt

- Freeze-frame overlay.
- Separate hotkey for silent OCR to clipboard.
- Paragraph mode vs line mode.
- Collapsible result cards, one per engine, each with its own retry.
- Source text is editable and can be re-translated.
- In-place overlay translation, later as phase 2.

## Capture

- Use GDI BitBlt for a single frozen frame. It takes about 15–40 ms for a 4K screen (U) and needs no setup.
- Do not use DXGI Desktop Duplication or Windows.Graphics.Capture for one-shot capture.
  - WGC draws a yellow border on Win10 and only delivers the first frame after a few vsyncs.
  - DXGI's first frame can be empty.
- Run the process as Per-Monitor-V2 DPI aware and work in physical pixels. Virtual-desktop coordinates can be negative.
- Use one overlay window per monitor. After showing it, check that its size matches the monitor.

## OCR

- **PP-OCRv6 small** is the default. It covers 50 languages, including zh, en and ja.

  | Model | det | rec | det acc. | rec acc. |
  |---|---|---|---|---|
  | v6 small (default) | 9.4 MiB | 20.2 MiB | 84.1 | 81.3 |
  | v6 tiny | 1.7 MiB | 4.3 MiB | n/a | n/a |
  | v5 mobile | 4.6 MiB | 15.8 MiB | n/a | n/a |

  - v6 tiny has no Japanese.
  - v5 mobile accuracy by language: zh 0.86, en 0.88, ja 0.76.
- With a warm session, a small crop should take about 50–200 ms (U).
- Rust options:
  - **oar-ocr 0.10.2** on **ort 2.0.0-rc.15** (ONNX Runtime 1.31). Pin both.
  - Avoid ocr-rs (MNN). It used about 400 MB at peak and took about 0.9 s.
  - Avoid paddle-ocr-rs, which is stale.
- Windows.Media.Ocr is a fast fallback.
  - It fails on very small regions, so upscale 2×.
  - Build the `SoftwareBitmap` from BGRA directly, not from a PNG.
- Vision LLMs are the most accurate on stylized or vertical text, but take 1–3 s. Offer them as an optional mode.

## Accuracy tricks

- **Upscaling and padding.** If text height is under 20 px, upscale 2–3× with Lanczos. Pad the image by 8–16 px.
- **Binarization.** Do not binarize input for neural OCR.
- **Inversion.** Invert light-on-dark images only for Windows OCR.
- **Line merging:**
  - Group lines into columns, then cluster them by vertical gap (less than about 1× line height) and left alignment.
  - Detect the end of a paragraph from a short last line, terminal punctuation or indentation.
  - Join CJK lines without a space and Latin lines with a space. Remove end-of-line hyphenation.
  - Keep line mode for UI text.
- **Language detection.** Kana means ja. Han-only text means zh. Then use whatlang on the rest. If the detected language equals the target, flip to the secondary target.
- **LLM prompting:**
  - Send numbered lines and ask for structured output.
  - Pass the foreground window title as context.
  - Say that the text may contain OCR errors.
  - Cache results by a hash of the text.

## Translation

| Engine | Free allowance / price | Notes |
|---|---|---|
| DeepL Free | 500k chars/month | |
| Microsoft F0 | 2M chars/month | Needs verification. |
| Tencent | 5M chars/month | |
| Alibaba | 1M chars/month | |
| Baidu | Unclear | Needs verification. |
| Claude Haiku 5.5 | $0.10 / $0.50 per M tokens | About $0.00008 per typical request. Disable thinking or use low effort to cut latency. |
| OpenAI-compatible base URL | Varies | Also covers Ollama for local/offline use. |

- Unofficial Google and Bing web endpoints are brittle. Keep them, but mark them "free, best effort".
- Skip Bergamot for v1.

## Sizes (U)

- The Tauri exe is about 4–8 MB.
- The ONNX Runtime DLL is about 12–20 MB, or roughly the same amount if linked statically.
- The models are about 30 MB.
- With the tray running and no webview, the app should sit at about 15–30 MB.
- A hidden WebView2 adds 50–90 MB. That is why the popup is lazy and gets destroyed when idle.
- OCR sessions add 100–200 MB. That is why the OCR engine unloads after being idle.
