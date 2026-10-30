#!/usr/bin/env node
// Builds the app icon set from design/icon*.svg into src-tauri/icons/.
//
//   node scripts/build-icons.cjs        (run from the repo root; needs `npm ci` in ui/)
//
// 16/24/32 px come from the hand-tuned, pixel-aligned SVGs (design/icon-{16,24,32}.svg).
// Larger sizes come from the 1024 master, rendered once and downsampled by halving: rendering
// the master directly at 48 px turns its grain into speckle.
//
// Outputs: icon.ico (16, 24, 32, 48, 64, 256; PNG-compressed entries), 32x32.png,
// 128x128.png, 128x128@2x.png and icon.png (512), plus design/icon-*.png previews.

const fs = require('node:fs');
const path = require('node:path');
const { createRequire } = require('node:module');

const ROOT = path.resolve(__dirname, '..');
const DESIGN = path.join(ROOT, 'design');
const OUT = path.join(ROOT, 'src-tauri', 'icons');
// Playwright lives in ui/node_modules (or globally); resolve it from there.
const { chromium } = createRequire(path.join(ROOT, 'ui', 'package.json'))('playwright');

async function render(page, svgFile, size) {
  const svg = fs.readFileSync(path.join(DESIGN, svgFile), 'utf8');
  return page.evaluate(
    async ({ svg, size }) => {
      const img = new Image();
      img.src = 'data:image/svg+xml;base64,' + btoa(unescape(encodeURIComponent(svg)));
      await img.decode();
      const c = document.createElement('canvas');
      c.width = c.height = size;
      c.getContext('2d').drawImage(img, 0, 0, size, size);
      return c.toDataURL('image/png');
    },
    { svg, size },
  );
}

/** Downsample a PNG data URL to `size` by repeated halving, then one final high-quality step. */
async function downsample(page, dataUrl, size) {
  return page.evaluate(
    async ({ dataUrl, size }) => {
      const img = new Image();
      img.src = dataUrl;
      await img.decode();
      let src = img;
      let w = img.width;
      while (w / 2 >= size) {
        const c = document.createElement('canvas');
        c.width = c.height = w / 2;
        const g = c.getContext('2d');
        g.imageSmoothingQuality = 'high';
        g.drawImage(src, 0, 0, w / 2, w / 2);
        src = c;
        w /= 2;
      }
      const c = document.createElement('canvas');
      c.width = c.height = size;
      const g = c.getContext('2d');
      g.imageSmoothingQuality = 'high';
      g.drawImage(src, 0, 0, size, size);
      return c.toDataURL('image/png');
    },
    { dataUrl, size },
  );
}

const png = (dataUrl) => Buffer.from(dataUrl.split(',')[1], 'base64');

/** ICO with PNG-compressed entries (Windows Vista and later). */
function ico(entries) {
  const header = Buffer.alloc(6 + 16 * entries.length);
  header.writeUInt16LE(0, 0);
  header.writeUInt16LE(1, 2);
  header.writeUInt16LE(entries.length, 4);
  let offset = header.length;
  entries.forEach(([size, data], i) => {
    const o = 6 + 16 * i;
    header.writeUInt8(size % 256, o);
    header.writeUInt8(size % 256, o + 1);
    header.writeUInt16LE(1, o + 4); // planes
    header.writeUInt16LE(32, o + 6); // bits per pixel
    header.writeUInt32LE(data.length, o + 8);
    header.writeUInt32LE(offset, o + 12);
    offset += data.length;
  });
  return Buffer.concat([header, ...entries.map(([, d]) => d)]);
}

(async () => {
  const browser = await chromium.launch();
  const page = await browser.newPage();
  await page.setContent('<!doctype html><html><body></body></html>');

  const master = await render(page, 'icon.svg', 1024);
  const sizes = {};
  for (const s of [16, 24, 32]) sizes[s] = png(await render(page, `icon-${s}.svg`, s));
  for (const s of [48, 64, 128, 256, 512]) sizes[s] = png(await downsample(page, master, s));
  await browser.close();

  fs.mkdirSync(OUT, { recursive: true });
  fs.writeFileSync(path.join(OUT, 'icon.ico'), ico([16, 24, 32, 48, 64, 256].map((s) => [s, sizes[s]])));
  fs.writeFileSync(path.join(OUT, '32x32.png'), sizes[32]);
  fs.writeFileSync(path.join(OUT, '128x128.png'), sizes[128]);
  fs.writeFileSync(path.join(OUT, '128x128@2x.png'), sizes[256]);
  fs.writeFileSync(path.join(OUT, 'icon.png'), sizes[512]);
  fs.writeFileSync(path.join(DESIGN, 'icon.png'), png(master));
  for (const s of [16, 24, 32]) fs.writeFileSync(path.join(DESIGN, `icon-${s}.png`), sizes[s]);
  console.log('icons written to', path.relative(ROOT, OUT));
})().catch((e) => {
  console.error(e);
  process.exit(1);
});
