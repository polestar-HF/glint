#!/usr/bin/env node
// Renders the popup and settings pages in light and dark with the mock backend and
// writes PNGs to docs/screenshots/. Run `npm run build` first.
//
//   node scripts/screenshots.cjs [filter]
//
// Segoe UI Variable and Microsoft YaHei UI only exist on Windows. To make Linux
// screenshots representative, set FONTS_DIR to a node_modules folder that contains
// @fontsource-variable/{open-sans,noto-sans-sc,noto-sans-jp}; they are mapped onto the
// Windows family names for the screenshot only (the app itself never loads web fonts).

const { chromium } = require('playwright');
const { spawn } = require('node:child_process');
const fs = require('node:fs');
const path = require('node:path');
const http = require('node:http');

const ROOT = path.resolve(__dirname, '..');
const OUT = path.resolve(ROOT, '../docs/screenshots');
const PORT = 4179;
const BASE = `http://localhost:${PORT}`;
const FONTS_DIR = process.env.FONTS_DIR;
const filter = process.argv[2] ?? '';

function standInFontCss() {
  if (!FONTS_DIR) return '';
  const map = [
    ['open-sans', 'Open Sans Variable', ['Segoe UI Variable Text', 'Segoe UI Variable Small', 'Segoe UI Variable Display', 'Segoe UI']],
    ['noto-sans-sc', 'Noto Sans SC Variable', ['Microsoft YaHei UI', 'Microsoft YaHei']],
    ['noto-sans-jp', 'Noto Sans JP Variable', ['Yu Gothic UI']],
  ];
  let css = '';
  for (const [pkg, fam, aliases] of map) {
    const file = path.join(FONTS_DIR, '@fontsource-variable', pkg, 'index.css');
    if (!fs.existsSync(file)) continue;
    const src = fs.readFileSync(file, 'utf8').replace(/url\(\.\/files\//g, `url(/__fonts/${pkg}/`);
    for (const alias of aliases) {
      let block = src.split(`'${fam}'`).join(`'${alias}'`);
      // Open Sans runs a little larger than Segoe UI; trim it to match the Windows metrics.
      if (pkg === 'open-sans') block = block.replace(/font-display: swap;/g, 'font-display: block;\n  size-adjust: 94%;');
      else block = block.replace(/font-display: swap;/g, 'font-display: block;');
      css += block;
    }
  }
  return css;
}

function waitForServer(url, ms = 15000) {
  const end = Date.now() + ms;
  return new Promise((resolve, reject) => {
    const tick = () =>
      http
        .get(url, (r) => (r.resume(), resolve()))
        .on('error', () => (Date.now() > end ? reject(new Error('preview server did not start')) : setTimeout(tick, 150)));
    tick();
  });
}

const popupScenes = [
  { name: 'popup-recognizing', q: 'scene=recognizing' },
  { name: 'popup-pending', q: 'scene=pending' },
  { name: 'popup-streaming', q: 'scene=streaming' },
  { name: 'popup-done', q: 'scene=done', hover: '.result.primary' },
  { name: 'popup-error', q: 'scene=error' },
  { name: 'popup-ratelimit', q: 'scene=ratelimit' },
  { name: 'popup-ocr', q: 'scene=ocr' },
  { name: 'popup-langmenu', q: 'scene=done', click: '.lang-btn >> nth=1' },
  { name: 'popup-edited', q: 'scene=done', edit: true },
  { name: 'popup-pinned-collapsed-toast', q: 'scene=error', pinCopy: true },
  { name: 'popup-notext', q: 'scene=notext' },
  { name: 'popup-mica', q: 'scene=error&mica' },
  // v0.2
  { name: 'popup-timings', q: 'scene=done&timings=1' },
  { name: 'popup-timings-cold', q: 'scene=error&timings=1&cold=1' },
  { name: 'popup-timings-ocr', q: 'scene=ocr&timings=1' },
];
const settingsScenes = [
  { name: 'settings-general', q: 'section=general' },
  { name: 'settings-hotkeys', q: 'section=hotkeys', record: true },
  { name: 'settings-ocr', q: 'section=ocr' },
  { name: 'settings-translate', q: 'section=translate', expand: 0 },
  { name: 'settings-translate-engines', q: 'section=translate', scroll: true, test: true },
  { name: 'settings-appearance', q: 'section=appearance' },
  { name: 'settings-about', q: 'section=about' },
  { name: 'settings-en', q: 'section=translate&ui=en-US' },
  // v0.2
  { name: 'settings-engines-multi', q: 'section=translate&engines=multi', reveal: '.page > .group:nth-of-type(2)', offset: 0 },
  { name: 'settings-engines-multi-config', q: 'section=translate&engines=multi', expandId: 'openai-7mf2ad', confirmDelete: true },
  { name: 'settings-add-service', q: 'section=translate&engines=multi', scroll: true, picker: true },
  { name: 'settings-perf', q: 'section=about', reveal: '.page > .group:nth-of-type(2)', offset: 0 },
  { name: 'settings-perf-system', q: 'section=about', reveal: '.system', offset: 40 },
  { name: 'settings-perf-empty', q: 'section=about&perf=empty', reveal: '.page > .group:nth-of-type(2)', offset: 0 },
];

async function main() {
  fs.mkdirSync(OUT, { recursive: true });
  const server = spawn('npx', ['vite', 'preview', '--port', String(PORT), '--strictPort'], { cwd: ROOT, stdio: 'ignore' });
  try {
    await waitForServer(BASE + '/popup.html');
    const browser = await chromium.launch();
    const fontCss = standInFontCss();
    for (const scheme of ['light', 'dark']) {
      const ctx = await browser.newContext({ deviceScaleFactor: 2, colorScheme: scheme, viewport: { width: 900, height: 900 } });
      if (FONTS_DIR) {
        await ctx.route('**/__fonts/**', (route) => {
          const [, pkg, file] = new URL(route.request().url()).pathname.match(/__fonts\/([^/]+)\/(.+)$/);
          route.fulfill({ path: path.join(FONTS_DIR, '@fontsource-variable', pkg, 'files', file), contentType: 'font/woff2' });
        });
      }
      for (const s of popupScenes) {
        if (filter && !s.name.includes(filter)) continue;
        await shoot(ctx, `${BASE}/popup.html?${s.q}`, `${s.name}-${scheme}`, fontCss, async (page) => {
          if (s.hover) await page.hover(s.hover);
          if (s.click) {
            await page.click(s.click);
            await page.waitForTimeout(250);
          }
          if (s.pinCopy) {
            await page.keyboard.press('Control+KeyP');
            await page.click('.result >> nth=1 >> [aria-expanded]');
            await page.click('.result >> nth=0');
            await page.keyboard.press('Control+KeyC');
            await page.mouse.move(0, 0);
            await page.waitForTimeout(200);
          }
          if (s.edit) {
            await page.click('textarea.src-text');
            await page.keyboard.press('Control+End');
            await page.keyboard.type(' Labels matter.');
            await page.mouse.move(0, 0);
          }
        });
      }
      for (const s of settingsScenes) {
        if (filter && !s.name.includes(filter)) continue;
        await shoot(ctx, `${BASE}/settings.html?${s.q}`, `${s.name}-${scheme}`, fontCss, async (page) => {
          if (s.record) {
            await page.click('.recorder >> nth=1');
            await page.keyboard.down('Control');
            await page.keyboard.press('KeyC');
            await page.keyboard.up('Control');
            await page.waitForTimeout(100);
          }
          if (s.expand != null) {
            await page.click(`.engine >> nth=${s.expand} >> .disclose`);
            await page.waitForTimeout(200);
          }
          if (s.scroll) await page.$eval('.page', (el) => el.scrollTo(0, el.scrollHeight));
          if (s.reveal) {
            // Scroll the page so `reveal` starts near the top (minus `offset` for its group title).
            await page.$eval(
              '.page',
              (el, [sel, off]) => {
                const t = el.querySelector(sel);
                if (t) el.scrollTo(0, t.getBoundingClientRect().top - el.getBoundingClientRect().top + el.scrollTop - 20 - off);
              },
              [s.reveal, s.offset ?? 40],
            );
          }
          if (s.expandId) {
            await page.click(`[data-engine-row="${s.expandId}"] .disclose`);
            await page.waitForTimeout(150);
            await page.$eval(`[data-engine-row="${s.expandId}"]`, (el) => el.scrollIntoView({ block: 'start' }));
            await page.$eval('.page', (el) => el.scrollBy(0, -12));
            if (s.confirmDelete) await page.click(`[data-engine-row="${s.expandId}"] .remove`);
            await page.waitForTimeout(150);
          }
          if (s.picker) {
            await page.click('.add');
            await page.waitForTimeout(250);
          }
          if (s.test) {
            await page.click('.engine >> nth=1 >> text=测试');
            await page.click('.engine >> nth=2 >> text=测试');
            await page.waitForTimeout(900);
            await page.$eval('.page', (el) => el.scrollTo(0, el.scrollHeight));
          }
          await page.mouse.move(0, 0);
        });
      }
      await ctx.close();
    }
    await browser.close();
  } finally {
    server.kill();
  }
}

async function shoot(ctx, url, name, fontCss, act) {
  const page = await ctx.newPage();
  page.on('pageerror', (e) => console.error(`[${name}] page error:`, e.message));
  page.on('console', (m) => m.type() === 'error' && console.error(`[${name}] console:`, m.text()));
  await page.goto(url);
  if (fontCss) await page.addStyleTag({ content: fontCss });
  await page.evaluate(() => document.fonts.ready);
  await page.waitForTimeout(400);
  await act(page);
  await page.waitForTimeout(250);
  const box = await page.locator('#app').boundingBox();
  const pad = 24;
  const file = path.join(OUT, `${name}.png`);
  await page.screenshot({
    path: file,
    animations: 'disabled',
    clip: { x: box.x - pad, y: box.y - pad, width: box.width + pad * 2, height: box.height + pad * 2 },
  });
  await page.close();
  console.log('wrote', path.relative(process.cwd(), file));
}

main().catch((e) => {
  console.error(e);
  process.exit(1);
});
