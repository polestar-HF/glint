# Glint — Design

Owner: UI engineer. Source of truth for the look of `ui/` and for the native overlay.
Tokens live in `ui/src/styles/tokens.css`; keep this file and that one in sync.

Screenshots of every state are in `docs/screenshots/` (light and dark, rendered from the mock IPC).

## 1. Principles

1. **It should feel like part of Windows.** Glint should read as a small, well-made Windows 11 utility, not as a web page in a frame. We use Fluent's materials (Mica, layer, card), Fluent's control shapes and Fluent's text colors. Where Fluent has an answer, we take it.
2. **Typography carries the hierarchy.** We use size, weight and the four text alphas before we reach for boxes, fills or color. One section of text is separated from the next by a hairline, not a card.
3. **One accent, used sparingly.** The accent marks *state*: focus, selection, on, primary, streaming. It never decorates. In the popup, a typical frame uses the accent in at most two places.
4. **Quiet motion.** Motion confirms that something happened and then gets out of the way. It runs 120–160 ms with decelerate easing, and never bounces. Things that are loading breathe slowly; nothing spins unless an action is actually in flight.
5. **The text is the product.** Recognized and translated text gets the best seat: the largest size, the full text color and room to breathe. Chrome stays at 12–13 px and secondary colors.
6. **Fast is a design feature.** No web fonts and no startup work. A static first frame is baked into `popup.html`, so the window paints its "recognizing" state immediately.

## 2. Anti-patterns we avoid

| We don't | We do instead |
|---|---|
| Purple, indigo or blue-violet gradients, gradient text | One flat accent (Ink `#1E6AC5`), with its hue held at 256°, well away from violet |
| Glassmorphism cards, glow, neon, colored drop shadows | The OS Mica material, translucent Fluent layers and 1 px hairlines; a shadow only on flyouts and the toast |
| Sparkle or magic-wand icons, "✨ AI" labels | Engines are named plainly ("Claude", "Google 翻译"); no engine gets a special badge |
| Emoji as icons | One icon family, qingicon duotone, at 16 px (§7) |
| Big rounded "bento" cards with soft 24 px shadows | Radii of 4, 6 and 8 px; settings groups are single hairline-bordered lists |
| Inter everywhere | Segoe UI Variable (Small, Text and Display cuts) with Microsoft YaHei UI and Yu Gothic UI for CJK |
| Pill badges and chips for metadata | Metadata is quiet inline text: `PaddleOCR · 86 ms`, `免费 · 尽力而为`, `首选` |
| Hero banners, marketing copy, empty-state illustrations | Direct labels; an empty state is one line of text plus an action |
| Spinners for streaming, bouncy springs, slide-in panels | A 2 px caret that breathes, skeleton lines that sit on the real line grid, 160 ms fades |
| Save buttons and toasts for every setting | Settings save on change (debounced 400 ms), with one quiet status line in the nav |

## 3. Color

### Accent: Ink `#1E6AC5`

The placeholder `#2F6FEB` is replaced by **`#1E6AC5`** ("Ink", 墨蓝), which is `oklch(0.53 0.159 256)`.

- **Hue.** At 256° it sits in the Windows blue family (the system default `#0078D4` is 251°). The placeholder was 262°, closer to the "SaaS blue" range where web blues drift toward violet.
- **Chroma.** 0.159, against the placeholder's 0.199. It reads calm on Mica and doesn't vibrate next to body text.
- **Contrast.** 5.35:1 on white and 5.17:1 on the light layer (`#FBFBFB`), so it passes AA as link text and with white text on top. The placeholder was a borderline 4.57:1.
- **Dark theme.** Derived in CSS rather than stored: `oklch(from accent 0.76 c×0.72 h)`, which gives `#80B4F9`. That has 7.6:1 contrast on `#202020`, and text on the accent is black, as in Fluent dark.
- **Overlay.** At mid luminance (L 0.53), the 1 px selection border stays visible over both the dimmed surroundings and bright content.

`config.ui.accent` stores the light value. The UI derives the dark value. The native overlay uses the stored value as is.

The Appearance page offers six curated alternatives, all at the same lightness (L ≈ 0.53–0.56, ≥ 4.8:1 on white), so swapping one in never breaks contrast:

| id | 名称 | hex | dark (derived) |
|---|---|---|---|
| ink (default) | 墨蓝 | `#1E6AC5` | `#80B4F9` |
| teal | 青碧 | `#017B80` | `#7FBEC1` |
| pine | 松绿 | `#307E4B` | `#8BC098` |
| ochre | 赭石 | `#A4631F` | `#D8A67A` |
| cinnabar | 朱砂 | `#BB3E26` | `#F39480` |
| graphite | 石墨 | `#51565C` | `#ADB1B7` |

A custom hex value is also accepted.

### Neutrals and materials

Fluent's alpha-based values are used, so the same tokens work over Mica, over a layer and over a solid fallback.

| Token | Light | Dark | Use |
|---|---|---|---|
| `--bg-base` | `#F3F3F3` | `#202020` | Solid window background (`.no-mica` only) |
| `--bg-window` | transparent | transparent | Window background; Mica shows through |
| `--bg-layer` | `#FFFFFF` 50 % | `#3A3A3A` 30 % | Popup body, settings page |
| `--bg-card` | `#FFFFFF` 70 % | `#FFFFFF` 5.1 % | Settings groups |
| `--bg-control` | `#FFFFFF` 70 % | `#FFFFFF` 6.1 % | Buttons, fields, keycaps |
| `--bg-subtle-hover` | `#000000` 3.7 % | `#FFFFFF` 6.1 % | Hover on transparent things |
| `--bg-flyout` | `#FCFCFC` | `#2C2C2C` | Menus, dropdowns |
| `--bg-inverse` | `#2B2B2B` | `#F0F0F0` | Toast |
| `--skeleton` | `#000000` 6 % | `#FFFFFF` 7 % | Loading lines |
| `--stroke-divider` | `#000000` 8 % | `#FFFFFF` 8.4 % | Hairlines between sections and rows |
| `--stroke-card` | `#000000` 5.8 % | `#000000` 10 % | Group borders, page edge |
| `--stroke-control` / `-bottom` | 5.8 % / 16.2 % black | 7 % / 9.3 % white | Control border with Fluent's darker bottom edge |
| `--text-1` | `#000000` 89.6 % | `#FFFFFF` | Primary text, the primary translation |
| `--text-quiet` | `#000000` 78 % | `#FFFFFF` 86 % | Secondary translations |
| `--text-2` | `#000000` 60.6 % | `#FFFFFF` 78.6 % | Labels, descriptions, source text at rest |
| `--text-3` | `#000000` 44.6 % | `#FFFFFF` 54.4 % | Meta (engine, ms), placeholders |
| `--text-4` | `#000000` 36.1 % | `#FFFFFF` 36.3 % | Disabled, scrollbar thumb |
| `--critical` | `#C42B1C` | `#FF99A4` | Error icon and text |
| `--success` | `#0F7B0F` | `#6CCB5F` | Engine test passed |
| `--caution` | `#9D5D00` | `#E3B341` | "需要 API Key" on an enabled engine |

**Theme.** By default the theme follows the system through `color-scheme: light dark`, and every color token is written as `light-dark()`. Setting `config.ui.theme` adds `data-theme="light|dark"` on `<html>`, and the UI calls `set_theme` on the window, which keeps the Mica material and the WebView's `prefers-color-scheme` in step. An inline boot script applies the last-known theme, accent and font size from `localStorage` before first paint, so there is no flash.

**Mica.** The page background is transparent, and surfaces are translucent layers over it. The `.no-mica` class swaps in solid `--bg-base`. It is set when:
- the URL has `?mica=0` or `?nomica`; or
- `navigator.userAgentData` reports Windows platformVersion < 13 (Windows 10).

## 4. Typography

No web fonts are loaded. The families are:

| Token | Stack |
|---|---|
| `--font-small` | `"Segoe UI Variable Small", "Segoe UI", <cjk>`, for 12 px and below |
| `--font-text` | `"Segoe UI Variable Text", "Segoe UI", <cjk>`, for 13–18 px |
| `--font-display` | `"Segoe UI Variable Display", "Segoe UI", <cjk>`, for 20 px and above |
| `<cjk>` | `"Microsoft YaHei UI", "Microsoft YaHei", "Yu Gothic UI", …, sans-serif` |
| `--font-content` | Recognized and translated text: `--font-text` for Latin content; **CJK-first** when the content is `lang="zh|ja|ko"`, so `“” — …` take their full-width forms |

- **Content language.** Content elements carry `lang`: `zh-CN`, `zh-TW`, `ja`, `ko` or `en`. `:lang(ja)` switches the CJK face to Yu Gothic UI, `zh-TW` to Microsoft JhengHei UI and `ko` to Malgun Gothic. Japanese never renders with Chinese glyph forms.
- **Weights.** Only 400 and 600 are used. Faux bold is disabled (`font-synthesis-weight: none`).
- **Numbers.** Elapsed times use tabular figures (`.tnum`).

**Scale (size / line height):**

| Role | Size | Line | Weight | Where |
|---|---|---|---|---|
| Caption | 12 | 16 | 400 / 600 | Meta, section labels (600), descriptions |
| Control | 13 | 20 | 400 | Popup strip, language menu, segmented control |
| Body | 14 | 20 | 400 | Settings rows and controls |
| Content | `ui.fontSize` (14) | ×1.6, rounded to 2 px (22) | 400 | Source text, secondary translations |
| Content, primary | `ui.fontSize` + 1 | ×1.6, rounded to 2 px (24) | 400 | First engine's translation |
| Subtitle | 20 | 28 | 600 Display | Settings page title |
| Title | 28 | 36 | 600 Display | About wordmark |

## 5. Space, size and radius

- **Grid.** Everything sits on a 4 px grid: 4, 8, 12, 16, 20, 24, 32.
- **Alignment rule.** A glyph aligns to the content edge, not its hit area. Icon buttons bleed into the gutter so that their 16 px glyph box lines up with the text edge. That is why the popup strip has a right padding of 10 px (28 px buttons, 6 px inset) and section headers use `margin-right: -4px` (24 px buttons). The language button's text starts exactly at the 16 px gutter.
- **Radii:**
  - 4 px for controls, keycaps, list items and the toast.
  - 6 px for settings groups.
  - 8 px for flyouts and windows (the OS draws the window corner).
- **Popup sizes:**
  - 420 px wide; the gutter is 16 px.
  - Title strip and OCR footer: 36 px.
  - Section header: 24 px.
  - Icon buttons: 28 px in the strip, 24 px in sections.
  - Section padding: 8 px top, 12 px bottom.
- **Settings sizes:**
  - The window is 760 × 540, with a 196 px nav and a 36 px nav item.
  - Page padding is 20 / 28 / 32 px.
  - Rows are at least 56 px tall (44 px when compact).
  - Controls are 32 px tall (small buttons 28 px).
  - The gap between groups is 24 px.

## 6. Elevation and materials

| Level | Surface | Treatment |
|---|---|---|
| 0 | Window | Mica (`--bg-window` transparent) or `--bg-base` |
| 1 | Popup body / settings page | `--bg-layer`, with a 1 px divider at the seam. The settings page has an 8 px top-left radius and a hairline on its top and left edges (the WinUI NavigationView content area) |
| 2 | Settings group | `--bg-card` plus a 1 px `--stroke-card`, radius 6, **no shadow** |
| 3 | Flyout (language menu, dropdown) | `--bg-flyout`, 1 px `--stroke-flyout`, radius 8, `0 8px 16px` at 14 % / 26 % |
| 3 | Toast | `--bg-inverse`, radius 4, `0 2px 8px` at 18 % |

## 7. Iconography

- **Family.** All UI icons come from [qingicon](https://qingicon.com) (QingCloud, Apache-2.0; see `third_party/qingicon`). Do not mix in glyphs from other sets: one family keeps weight and corner shapes consistent.
- **Grid.** qingicon draws filled shapes on a 48 × 48 grid. Glint always renders them at 16 px (1 px = 3 units), so no stroke thins out when scaled.
- **Two variants, two meanings.**
  - **Duotone** is the resting look. The primary layer is `currentColor`; the secondary layer is the same color at `--icon-tone` (0.45) opacity, so it follows the text color, hover and theme. qingicon's own fixed gray (`#b6c2cd`) is not used.
  - **Fill** (solid) means "on": the pinned pin, and the current page in the settings nav (`<Icon solid>`).
- **Smaller glyphs.** `chevron-small` (the language and select menus) is `chevron-down` inset by 8 units in the same 16 px box, not a smaller box.
- **Source.** `scripts/build-ui-icons.cjs` extracts the glyphs listed in its `MAP` from the `@pitrix/qingicon-react` sprite and generates `ui/src/lib/icons.ts`. Do not edit that file by hand. The set, as Glint name ← qingicon name:
  - chevron-down, chevron-up, chevron-small ← chevron-down
  - swap ← left-right, copy, close, pin, check, alert ← warning-2, info ← information
  - reset ← go-back, enter ← go-back-turn-2, grip ← drag, external ← share-box
  - Settings nav: gear ← gear-2 (常规), keyboard (快捷键), scan ← scan-code (识别), translate ← language-switch-2 (翻译), palette (外观), info (关于)
  - eye, eye-off, plus ← add, trash

## 8. Motion

| Token | Value | Use |
|---|---|---|
| `--dur-1` | 120 ms | Hover and press color changes, exits (toast out) |
| `--dur-2` | 160 ms | Entrances (flyout, toast in, test result), toggle knob, reorder (`flip` 140 ms) |
| `--ease-out` | `cubic-bezier(0.1, 0.9, 0.2, 1)` | Things arriving (Fluent "decelerate") |
| `--ease-in` | `cubic-bezier(0.7, 0, 1, 0.5)` | Things leaving |
| `--ease-std` | `cubic-bezier(0.33, 0, 0.67, 1)` | Point-to-point state changes |

- **Looping animations:**
  - The skeleton breathes, with opacity going 1 → 0.45 over 1.2 s, alternating.
  - The streaming caret breathes, dipping to 0.2 opacity on a 1.1 s cycle.
  - The test spinner rotates once every 0.8 s, and only while `test_engine` is running.
- **Things that don't animate:**
  - Collapse in the popup is instant. Animating height would resize the native window across frames.
  - The overlay has no animation at all.
- **Reduced motion.** `prefers-reduced-motion: reduce` sets both durations to 0.

## 9. Components

### Popup (`popup.html`)

**Title strip** (36 px, on Mica, `data-tauri-drag-region` on the strip and its spacer):
- **Language pair** `[英语 自动 ⌄] ⇄ [简体中文 ⌄]`.
  - Each button is 28 px tall, 13 px text, with a `text-3` chevron.
  - When the source is `auto` and a language has been detected, the button shows the detected name followed by a `text-3` "自动".
  - Swap is disabled until a source language is known.
- **Icon buttons** on the right: pin (`aria-pressed`; solid and accent-colored when pinned), copy-all, close. All are neutral on hover; there is no red close.

**Language menu** (flyout):
- It opens under the strip, 8 px from the left, with a "源语言" / "目标语言" caption and a 3-column grid of 32 px items.
- The selected item gets a subtle fill and a 3 × 14 px accent indicator, as in Fluent lists.
- Arrow keys move through the grid. Enter picks an item. Esc closes the menu and returns focus to the button.
- While the menu is open, the popup reports `max(content, menu bottom + 8)` as its height, so the menu always fits.
- Picking the target language as the source swaps the pair.
- Language changes apply to this session only and are not saved; Settings holds the defaults. Each change calls `retranslate`. If OCR is still running, the change is queued and applied when the text arrives.

**Source section:**
- **Header.** "原文" (12/600, `text-2`), then meta (`PaddleOCR · 86 ms`; "正在识别…" while pending; "已编辑" after a re-translation of edited text), then actions.
- **Text.** A borderless `<textarea>` with `field-sizing: content`, so it auto-grows. It is `text-2` at rest and `text-1` while focused, with an accent caret. The section gets a 2 % wash while focused; there is no focus box.
- **Re-translating.** When the text differs from what was translated, a `↵ 重新翻译` text button appears (title `Ctrl+Enter`), and the old results drop to 50 % opacity until the new ones arrive.
- **Collapsing.** Collapsed, the section is a single line: label, meta, a one-line ellipsized preview and the chevron. Clicking the preview expands it.
- **Pending.** Two skeleton lines (100 %, 64 %).
- **OCR error.** An alert icon, "识别失败：…" and a `重新截图` action (`start_capture`).
- **No text.** "没有识别到文字" and `重新截图`.

**Result section** (one per engine, in config order):
- **Header.** Engine name (12/600, `text-2`), then elapsed (`412 ms`, `1.2 s`; tabular) once done. Copy and collapse appear on hover or focus, and always when the section is collapsed.
- **Primary engine** (the first one): content size + 1 at `text-1`. **Others:** content size at `text-quiet`. This is the only difference; labels look identical.
- **States:**
  - **Waiting** (OCR still running): one 56 % skeleton line.
  - **Pending** (`tr:start`, no text yet): two lines (100 %, 72 %) for the primary engine, one line (88 %) for the others.
  - **Streaming:** text followed by a 2 × 1.15 em accent caret.
  - **Done:** text and elapsed time.
  - **Error:** a single 24 px line with a 16 px critical-colored alert icon, the message (13 px, `text-2`, ellipsized, full text in the tooltip) and a `重试` text button aligned to the right edge.
- **Placeholders.** These come from the config's enabled engines. Any that never receive `tr:start` are pruned 600 ms after the first one that does.

**OCR mode:**
- The strip shows "文字识别 · 简体中文" in place of the language pair.
- The body is the editable text at `text-1`, with no header.
- A 36 px footer on Mica shows `PaddleOCR · 64 ms · 3 行` and a `文A 翻译` text button, which turns the session into a translation.

**Toast:**
- A 28 px inverse chip with a check icon and "已复制", bottom-center and 12 px above the edge.
- It enters in 160 ms (fade in, rising 4 px), holds for 1.4 s, then exits in 120 ms.
- It has `role="status"`.

**Keyboard:**

| Key | Action |
|---|---|
| Esc | Hide the popup (`popup_hide`); closes the language menu first if it is open |
| Ctrl+C, nothing selected | Copy the primary translation (the text, in OCR mode) |
| Ctrl+P | Toggle pin |
| Ctrl+Enter | Re-translate the edited source |

**Sizing:**
- The popup reports `strip + 1 + content (+ footer)` via `popup_resize`.
  - The value is measured with a ResizeObserver and coalesced to one call per animation frame, at most one every 50 ms.
  - It is capped at 70 % of `screen.availHeight`; beyond that the body scrolls.
  - Only width changes (a user resize) feed back into the height.

**Data flow on mount:**
1. Subscribe to all events, buffering anything that arrives early.
2. Call `get_config` and `get_session_snapshot` in parallel and apply both.
3. Replay the buffer.
   - Deltas carry `seq` (CR #3), so ones already folded into the snapshot are skipped. Without `seq`, a delta that the snapshot text already ends with is skipped.
   - A replayed `tr:start` never wipes text that is streaming.

Events whose `sessionId` is older than the current one are dropped. A `retranslate` whose `session:start` arrives before the command resolves keeps the edited text.

**Timings line (v0.2, `ui.showTimings`):**
- One extra meta row directly under the source header: 12/16 `--font-small`, `text-3`, tabular figures. Example: `截图 21 ms · 选区 34 ms · 识别 142 ms（检测 48 / 识别 81）· 首字 585 ms`.
- **Sources.** `session:start.timings` (capture, overlay), `ocr:result.timings` (load, det, rec) with `elapsedMs`, and the first result's `tr:done.firstTokenMs`; snapshot equivalents on catch-up. Absent parts are left out; a cold start adds `加载 1046` inside the parentheses.
- Inner stage values are bare integers (ms implied); headline values use `fmtMs` (`142 ms`, `1.2 s`).
- **Wrapping.** Each part is unbreakable and carries its trailing `·` (joined by a no-break space), so a wrap never starts a line with a dot.
- **No duplicates.** With the line on, the source header meta drops its total (`PaddleOCR` alone), and so does the OCR-mode footer.
- **Per engine.** Each result header shows `首字 585 ms · 1.1 s` in place of the bare elapsed time.
- Capture and OCR timings survive a `retranslate`, since they describe the same capture.
- In OCR mode the line sits 8 px under the text, above the footer.

### Settings (`settings.html`)

- **Navigation.**
  - Each item is 36 px tall, with a 16 px icon, a 12 px gap and a 14 px label. Hover gets a subtle fill The current page's icon switches to its solid variant.
  - The selected item gets a fill plus a 3 × 16 px accent indicator, which scales in over 160 ms.
  - ↑ and ↓ move between sections.
  - The bottom line shows the save state: "更改会自动保存", "正在保存…", "✓ 已保存" or "保存失败" in the critical color, with the error in a tooltip.
- **Group.** A 14/600 title with an optional right-aligned 12 px hint, then a card holding rows separated by hairlines.
- **Row.** A title (14/20) and description (12/16, `text-2`) on the left, the control on the right, 24 px apart. A validation message (12 px, critical color, with an icon) goes under the description.
- **Toggle** (Fluent ToggleSwitch):
  - The track is 40 × 20, radius 10.
  - Off: a 1 px `--stroke-strong` border with a 12 px `text-2` knob, which grows to 14 px on hover and stretches to 17 px while pressed.
  - On: an accent fill with an `--on-accent` knob that slides 20 px over 160 ms.
- **Select** (Fluent ComboBox):
  - The button is 32 px tall with a 12 px left padding and a `text-2` chevron.
  - The list opens with the selected item lying over the button; it is clamped to the window and scrolls past 360 px.
  - Items are 32 px tall, and the selected one gets the accent indicator.
  - Keys:
    - ↑/↓ on a closed select change the value directly.
    - Enter, Space, Alt+↓ or F4 open the list.
    - Esc closes it.
  - The list closes on scroll, resize or blur.
- **Segmented** (theme): a 2 px-padded track, 26 px segments and 13 px labels. The selected segment is white (or white 9 % in dark) with a control stroke. It is a radio group, and the arrow keys move the selection.
- **Radio row** (OCR engine): a 20 px ring that becomes an accent disc with an 8 px center dot when checked. The title is followed by a 12 px `text-2` tag ("本地 · 高精度"), with the description on the line below.
- **Key recorder:**
  - A 32 px field, at least 176 px wide, showing keycaps left-aligned.
  - **Keycap:** 22 px tall, 1 px stroke, a 2 px bottom edge and 12 px text. `Super` is shown as **Win**.
  - **Recording:** clicking (or pressing Enter) starts it. The field turns input-white with a 2 px accent bottom edge and shows the modifiers being held live, with the row hint "Esc 取消 · Backspace 清除".
  - **Validation**, local first:
    - At least one modifier is required, except for F13–F24, PrintScreen and Pause.
    - Shift + letter is rejected.
    - Windows-reserved combinations are rejected (Win+L, Alt+Tab, Win+Shift+S…).
    - Editing shortcuts are rejected (Ctrl+C/V/X/Z/A/S…).
    - A shortcut already used by another Glint action is rejected.
  - **Backend checks.** After local validation, `check_hotkey` checks the shortcut with the OS (CR #4). While recording, `hotkeys_suspend(true)` keeps Glint's own shortcuts from firing (CR #5).
  - **Reset.** A reset icon appears when the value differs from the default.
- **Engine row:**
  - **Layout:** a grip (`text-4`, `text-2` on hover), then the name (14 px), then the subline (12 px `text-3`).
  - **Subline:** "免费 · 尽力而为" for free engines, the model or host for configured ones, and "需要 API Key" in the caution color for an engine that is enabled but not configured.
  - **Primary mark:** the first enabled engine shows "首选" in accent text after its name.
  - **Actions:** `测试` and `配置 ⌄` are *subtle* buttons with no border or fill until hovered, so seven rows don't become a wall of boxes. They are followed by the toggle.
  - **Test result:** shown inline below the subline, with a check plus the result plus ms in the success color, or an alert plus the message in the critical color.
  - **Reordering:** by pointer drag on the grip (live reorder, `flip` 140 ms, the dragged row lifted with the flyout shadow) or with ↑ and ↓ while the grip is focused.
  - **Configuration** (disclosure): a 96 px label column and 32 px fields, plus a multi-line field for the prompt. Fields by kind:
    - anthropic: key, model, base URL, prompt
    - openai: base URL, key, model, prompt
    - deepl: key
    - youdao: app ID, app secret
    - baidu: app ID, secret key
    - google and bing: a note only
    - every kind: display name
  - **Secret fields** have a reveal toggle (eye icon). Hints are 12 px `text-3`.
- **Text field:** 32 px, radius 4, control fill and stroke with a darker bottom edge. On focus it turns input-white with a 2 px accent bottom edge (Fluent TextBox), and the focus ring is suppressed.
- **About:** the wordmark "Glint" (28/600 Display) takes the place of the page title, followed by the tagline and version. Below are links (rows with an external-link icon, opened through `tauri-plugin-opener`), the performance card, open-source licenses and the config file path (selectable, monospace).

#### v0.2: multiple LLM services

- **"添加大模型服务"** is the last row of the engine card, not a separate button below it. The plus icon sits in the grip column and the label on the engine-name edge, so it reads as "a new row goes here". It is accent-colored text with a subtle hover fill.
- **Preset picker** (flyout, 320 px, radius 8, flyout shadow):
  - Two groups with 12 px `text-3` captions ("OpenAI 兼容", "Anthropic"), separated by a hairline that runs edge to edge.
  - Items are 32 px: name on the left (14 px `text-1`), endpoint host right-aligned (12 px `text-3`). Local services show "本地，无需 Key"; "自定义" shows "填写接口地址".
  - It opens downward when there is room (≥ 320 px), otherwise upward, and is clamped to the window. ↑/↓/Home/End move, Enter picks, Esc closes and returns focus. A scroll that lands within 200 ms of opening (the click scrolling the button into view) is ignored.
- **After picking:** a new entry (`<kind>-<6 × [a-z0-9]>`, `enabled: true`, name and endpoint from the preset) is appended. Its configuration panel opens, the row scrolls into view, and focus goes to the first thing the user must type: the base URL for "自定义", otherwise the first empty required field (key, then model).
- **User-added rows:**
  - Name, then a quiet protocol tag (12 px `text-3`: "OpenAI 兼容" / "Anthropic"), then "首选" if primary.
  - Secondary line: `host · model` (the default host when the base URL is empty). A missing-field hint follows on the same line in the caution color: "需要 API Key", "需要填写模型名称", or "需要 API Key 和模型名称".
  - The display name can't be emptied: user-added entries always keep a name.
  - Delete lives at the end of the configuration panel: a subtle critical-colored "删除此服务" with a trash icon. Clicking it turns the row into "删除「通义千问」？ [删除] [取消]" with focus on 取消; Esc cancels. No modal.
- **Validation** mirrors the backend:
  - openai: a key is required on the official endpoint (no base URL, or `api.openai.com`) and on hosted presets that need one. A model is required whenever the base URL is custom.
  - anthropic: a key is required unless a base URL is set.
  - A required field that is still empty gets a caution-colored bottom edge; "可选" appears only on fields that really are optional for that endpoint.
- Built-in rows are unchanged and can't be deleted. Drag and ↑/↓ reorder cover all rows alike.

#### v0.2: 性能数据 (About)

- One group titled "性能数据", with "最近 128 次 · 自 10月21日 起" as the right-aligned hint. Rows are separated by hairlines:
  1. The "在弹窗中显示耗时" toggle (`ui.showTimings`).
  2. **Timings table.** The first header cell doubles as the caption ("耗时（毫秒）", 12/600 `text-2`); the other headers are 12 px `text-3`, right-aligned. Rows are 28 px at 13 px; numbers are tabular with thousands separators. 中位数 is `text-1`, P90 `text-2`, 最大 `text-3`. Metrics follow pipeline order: hotkey → capture → overlay → OCR warm/cold → load → det → rec. No row rules.
  3. **Translation table.** Columns 次数, 失败, 中位数, P90, 首字. A non-zero 失败 is in the critical color; a missing first-token value is "—".
  4. **System.** Key/value pairs on the 96 px label column used by the engine fields; the values are selectable.
  5. **Actions** on the right: subtle "清空" (inline confirm "清空全部统计？ [清空] [取消]") and a standard "复制诊断信息" button with a copy icon. Copying shows the same inverse toast as the popup.
- **Empty state:** a single 13 px `text-2` line, "使用几次截图翻译后，这里会显示统计。" The system block and the copy action stay, since diagnostics are still useful.

## 10. Writing

- **Language.** zh-CN first, with en-US as a mirror. Both live in one dictionary in `ui/src/lib/i18n.svelte.ts`; there is no i18n library.
- **Voice.** Short, plain verbs: "复制", "重试", "重新截图". No exclamation marks and no "智能" or "AI" adjectives.
- **Descriptions.** One line, saying what the setting changes for the user, not how it is built.
- **Free engines** are labelled "免费 · 尽力而为" / "Free · Best effort", and their config note says the endpoint is unofficial.

## 11. Native overlay visual spec (refined)

This refines the initial spec in ARCHITECTURE.md. The current implementation (`glint-overlay/src/overlay/layout.rs`) already matches it except where marked **change**. All sizes are logical px × monitor scale, rounded to whole physical pixels.

| Element | Spec |
|---|---|
| Accent | `OverlayOptions.accent` = `config.ui.accent`, used as is (default **change**: `0x1E6AC5`) |
| Dim layer | `#000000` at 45 % over the frozen frame. The selection, and a snapped window, are not dimmed |
| Selection border | 1 px accent, drawn **outside** the selection so no captured pixel is covered |
| Handles | Eight 6 × 6 squares: 1 px accent edge, white `#FFFFFF` center, centered on the border. **Change:** hide them while the selection is under 24 × 24 logical px, so tiny selections stay readable |
| Window snap (before the drag starts) | 2 px accent outline drawn **inside** the window rect, no fill, interior undimmed. Confirmed |
| Size chip | `#1F1F1F` at 90 %, text `#F2F2F2`, "Segoe UI Variable" (Small optical size) at 12 px / 400, padding 4 × 8, radius 4. Placed 4 px above the top-left corner, or below it if there is no room. Text `1280 × 720`: U+00D7 with plain spaces, in physical px. **Change:** enable tabular figures (`DWRITE_FONT_FEATURE_TAG_TABULAR_FIGURES`) so the chip doesn't jitter during a drag |
| Magnifier | 15 × 15 cells at 8 px = 120 × 120, with a 1 px `#1F1F1F` at 90 % frame. Grid is 1 physical px `#808080` at 35 %. The center row and column are tinted with the accent at 35 %. The center cell has a 1 px outline, black or white by luminance. Placed 20 px down-right of the cursor and flipped at monitor edges |
| Color line | A chip under the magnifier, 4 px gap, as wide as the box, same chip style. It holds a 10 px swatch with a 1 px `#F2F2F2` at 80 % frame, then a 6 px gap, then `#RRGGBB` in 12 px |
| Cursor | Crosshair (`IDC_CROSS`) |
| Motion | None |

## 12. Integration notes (for the lead)

- **Popup window:**
  - `decorations: false`, `transparent: true`, `shadow: true` (on Windows 11 this gives the rounded corners and the 1 px border, so the page draws no window frame).
  - `resizable: true`, `maximizable: false` (the drag region double-click would otherwise maximize), `skipTaskbar: true`, `alwaysOnTop: true`.
  - Width 420, initial height about 160.
  - `windowEffects: { effects: ["mica"] }`. On Windows 10, or if the effect fails, load `popup.html?mica=0`.
- **Settings window:**
  - `decorations: true`, `transparent: true`, `windowEffects: { effects: ["mica"] }`.
  - Size 760 × 540, minimum about 640 × 440.
  - Title: the page sets it to "Glint 设置" / "Glint Settings".
- **CSP.** Use `default-src 'self' ipc: http://ipc.localhost; script-src 'self'; style-src 'self'; img-src 'self' data:`.
  - Each page has one small inline boot script. Tauri hashes inline scripts into the CSP automatically, so do not set `dangerousDisableAssetCspModification`.
  - No inline `style` attributes ship in the markup. Svelte's `style:` directives go through the CSSOM (`style.cssText` / `setProperty`), which CSP allows, so `'unsafe-inline'` is not needed.
- **Capabilities:**
  - `core:default`
  - `core:window:allow-set-theme`, `core:window:allow-start-dragging`
  - `opener:allow-open-url`, from `tauri-plugin-opener` (used by About)
  - `core:app:allow-version`, which is in `core:app:default`
- **Theme.** Apply the system theme when the theme is `system`; the UI calls `set_theme` itself on override.
- **Build.** `npm ci && npm run build` writes `ui/dist/{popup,settings}.html` and `ui/dist/assets/*`. The mock chunk (`assets/mock-*.js`) is only fetched outside Tauri.

## 13. Dev preview

- **Running it.** `npm run dev`, then open `/popup.html` or `/settings.html`. Without `__TAURI_INTERNALS__`, the page loads the mock IPC (`src/lib/mock.ts`) and draws a stand-in Windows 11 frame and title bar (`html.preview`).
- **Popup scenes.** Pass `?scene=` with one of:
  - `live` (the default)
  - `recognizing`, `pending`, `streaming`, `done`, `error`
  - `ocr`, `notext`, `ocrerror`
- **Other options:**
  - `?freeze=<ms>` freezes the timeline at any point.
  - `?mica` simulates Mica.
  - `?ui=en-US` switches to English; `?font=16` changes the content size; `?theme=dark` forces a theme.
  - `settings.html?section=translate` opens a given settings section.
  - v0.2: `popup.html?timings=1` turns the timings line on, and `&cold=1` adds a model load. `settings.html?engines=multi` adds two user-added services (one missing its model); `?perf=empty` starts with no statistics.
- **Screenshots.** `npm run build && node scripts/screenshots.cjs` writes `docs/screenshots/*.png`. Linux has no Segoe UI or YaHei, so set `FONTS_DIR` to a `node_modules` folder containing `@fontsource-variable/{open-sans,noto-sans-sc,noto-sans-jp}`. These stand in for Segoe UI Variable and YaHei UI / Yu Gothic UI, for the screenshot only. On Windows, the real faces render tighter. The committed PNGs are 2× and were optimized losslessly with oxipng. Don't quantize them: palette reduction merges the dark-theme layers, which differ by only about 8 levels.
- **Budget.** Production JS, gzipped: the popup is about 38 KB (7.6 KB page + 30 KB shared runtime), settings about 49 KB (18.7 KB page + 30 KB shared), against a 60 KB budget. CSS is about 4 KB for the popup and about 7 KB for settings.
