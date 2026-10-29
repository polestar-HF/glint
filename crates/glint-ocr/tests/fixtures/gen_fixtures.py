#!/usr/bin/env python3
"""Regenerates the OCR test fixtures (PNG + expected.json) with Pillow.

Not run by `cargo test`; the generated files are committed. Fonts are the
ones available in the Linux build container (Inter, DejaVu, WenQuanYi Zen Hei,
IPAGothic), so re-running on another machine gives slightly different pixels.

Each case lists the physical lines as drawn (`lines`) and the text a perfect
OCR + paragraph merge should return (`expected`). `\n` separates paragraphs.
"""
import json
import os
from PIL import Image, ImageDraw, ImageFont

HERE = os.path.dirname(os.path.abspath(__file__))
INTER = "/usr/share/fonts/opentype/inter/Inter-Regular.otf"
DEJAVU = "/usr/share/fonts/truetype/dejavu/DejaVuSans.ttf"
WQY = "/usr/share/fonts/truetype/wqy/wqy-zenhei.ttc"
IPA = "/usr/share/fonts/opentype/ipafont-gothic/ipag.ttf"


def render(name, lines, font_path, size, size_px, bg, fg, line_gap, margin=(14, 12),
           columns=None):
    """lines: list of str (single column) or None when `columns` is used."""
    font = ImageFont.truetype(font_path, size)
    img = Image.new("RGB", size_px, bg)
    d = ImageDraw.Draw(img)
    if columns is None:
        columns = [(margin[0], lines)]
    for x, col_lines in columns:
        y = margin[1]
        for ln in col_lines:
            d.text((x, y), ln, font=font, fill=fg)
            y += size + line_gap
    path = os.path.join(HERE, name + ".png")
    img.save(path, optimize=True)
    return path


cases = {}

# 1. English paragraph, with an end-of-line hyphenation ("transla-" / "tion").
en = [
    "Glint recognizes text on your screen in a fraction of a",
    "second. Select any region with the mouse, and the transla-",
    "tion appears right next to it, so you never have to switch",
    "windows or retype a single word.",
]
render("en_paragraph", en, INTER, 16, (600, 112), (255, 255, 255), (28, 28, 30), 7)
cases["en_paragraph"] = dict(
    lines=en,
    expected="Glint recognizes text on your screen in a fraction of a second. "
    "Select any region with the mouse, and the translation appears right next to it, "
    "so you never have to switch windows or retype a single word.",
    lang="en",
)

# 2. Chinese paragraph.
zh = [
    "截图翻译工具可以在几毫秒内识别屏幕上的文字。选择任意区域，",
    "译文便会显示在旁边，无需切换窗口，也不用手动输入任何内容。",
    "识别结果支持中文、英文、日文等多种语言。",
]
render("zh_paragraph", zh, WQY, 18, (600, 108), (250, 250, 247), (30, 30, 30), 8)
cases["zh_paragraph"] = dict(lines=zh, expected="".join(zh), lang="zh")

# 3. Japanese with kana.
ja = [
    "スクリーンショットから文字を読み取り、すぐに翻訳します。",
    "ひらがなとカタカナ、そして漢字が混ざった文章でも認識できます。",
]
render("ja_kana", ja, IPA, 18, (600, 84), (255, 255, 255), (20, 20, 20), 8)
cases["ja_kana"] = dict(lines=ja, expected="".join(ja), lang="ja")

# 4. Mixed Chinese / English.
mixed = [
    "Glint 支持 Windows 10 和 Windows 11 系统，默认快捷键为",
    "Alt+Q，可以在 Settings 中修改。Version 0.1.0 已于 2026",
    "年 10 月发布。",
]
render("mixed_zh_en", mixed, WQY, 18, (600, 108), (255, 255, 255), (25, 25, 25), 8)
cases["mixed_zh_en"] = dict(
    lines=mixed,
    expected="Glint 支持 Windows 10 和 Windows 11 系统，默认快捷键为Alt+Q，可以在 Settings "
    "中修改。Version 0.1.0 已于 2026年 10 月发布。",
    lang="zh",
)

# 5. Tiny 11 px text (DejaVu renders cleanly at this size).
tiny = [
    "The quick brown fox jumps over the lazy dog.",
    "Pack my box with five dozen liquor jugs.",
]
render("tiny_11px", tiny, DEJAVU, 11, (320, 46), (255, 255, 255), (0, 0, 0), 4, margin=(8, 7))
cases["tiny_11px"] = dict(
    lines=tiny,
    expected="The quick brown fox jumps over the lazy dog. Pack my box with five dozen liquor jugs.",
    lang="en",
)

# 6. White-on-dark UI list (should stay in line mode).
ui = ["General", "Appearance", "Keyboard Shortcuts", "Translation Engines", "About Glint"]
render("dark_ui", ui, INTER, 14, (260, 150), (30, 30, 30), (240, 240, 240), 12, margin=(16, 14))
cases["dark_ui"] = dict(lines=ui, expected="\n".join(ui), lang="en")

# 7. Two columns of text (left column must be read first).
left = [
    "Fast offline recognition",
    "keeps every screenshot on",
    "your own computer, with no",
    "upload and no account.",
]
right = [
    "Translate with the engine",
    "you trust: a free web service,",
    "your own API key, or a local",
    "model running on this PC.",
]
render("two_column", None, INTER, 15, (600, 110), (255, 255, 255), (20, 20, 20), 6,
       columns=[(18, left), (326, right)])
cases["two_column"] = dict(
    lines=left + right,
    expected=" ".join(left) + "\n" + " ".join(right),
    lang="en",
)

with open(os.path.join(HERE, "expected.json"), "w", encoding="utf-8") as f:
    json.dump(cases, f, ensure_ascii=False, indent=2)
    f.write("\n")
print("wrote", len(cases), "fixtures")
