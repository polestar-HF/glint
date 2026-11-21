// Applies appearance settings to <html>. The inline boot script in each page applies
// the cached copy before first paint; this module keeps it current afterwards.

import { api } from './ipc';
import type { Config } from './types';

export const DEFAULT_ACCENT = '#0E8CB8';
const CACHE_KEY = 'glint.ui';

export const ACCENTS = [
  { id: 'prism', hex: '#0E8CB8' },
  { id: 'ink', hex: '#1E6AC5' },
  { id: 'teal', hex: '#017B80' },
  { id: 'pine', hex: '#307E4B' },
  { id: 'ochre', hex: '#A4631F' },
  { id: 'cinnabar', hex: '#BB3E26' },
  { id: 'graphite', hex: '#51565C' },
] as const;

export function normalizeHex(v: string | null | undefined): string | null {
  if (!v) return null;
  let s = v.trim().replace(/^#/, '');
  if (/^[0-9a-f]{3}$/i.test(s)) s = s.replace(/./g, (c) => c + c);
  return /^[0-9a-f]{6}$/i.test(s) ? `#${s.toUpperCase()}` : null;
}

export function clampFontSize(n: number): number {
  return Math.min(22, Math.max(12, Math.round(Number(n) || 14)));
}

let lastTheme: string | null = null;

export function applyUi(cfg: Config) {
  const root = document.documentElement;
  const { theme, accent, fontSize } = cfg.ui;
  if (theme === 'light' || theme === 'dark') root.dataset.theme = theme;
  else delete root.dataset.theme;
  root.style.setProperty('--accent-base', normalizeHex(accent) ?? DEFAULT_ACCENT);
  root.style.setProperty('--content-size', `${clampFontSize(fontSize)}px`);
  root.lang = cfg.general.uiLanguage === 'en-US' ? 'en-US' : 'zh-CN';

  if (theme !== lastTheme) {
    lastTheme = theme;
    api.setWindowTheme(theme).catch(() => {});
  }
  try {
    localStorage.setItem(
      CACHE_KEY,
      JSON.stringify({ theme, accent: normalizeHex(accent), fontSize: clampFontSize(fontSize), lang: root.lang }),
    );
  } catch {
    /* storage unavailable: the boot script falls back to defaults */
  }
}
