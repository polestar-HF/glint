// Settings state. Module-level $state so every section can edit it directly.
// Any change is saved after a short pause; there is no Save button.

import { api, on } from '../lib/ipc';
import { normalizeConfig } from '../lib/engines';
import { setLocale } from '../lib/i18n.svelte';
import { applyUi } from '../lib/theme';
import type { Config } from '../lib/types';

export type SaveState = 'idle' | 'saving' | 'saved' | 'failed';

export const store = $state<{ config: Config | null; save: SaveState; error: string }>({
  config: null,
  save: 'idle',
  error: '',
});

let lastSaved = '';
let timer: ReturnType<typeof setTimeout> | undefined;
let savedTimer: ReturnType<typeof setTimeout> | undefined;

function adopt(c: Config) {
  const cfg = normalizeConfig(c);
  lastSaved = JSON.stringify(cfg);
  store.config = cfg;
}

export async function load() {
  // Other windows (the popup's language menu, the tray) may change config too.
  await on('config:changed', (c) => {
    if (timer) return; // our own edit is pending; it wins
    if (JSON.stringify(normalizeConfig(c)) !== JSON.stringify(store.config)) adopt(c);
  });
  adopt(await api.getConfig());
}

/** Called from an effect whenever the config changes (deeply). */
export function onConfigChanged() {
  const c = store.config;
  if (!c) return;
  setLocale(c.general.uiLanguage);
  applyUi(c);
  const json = JSON.stringify(c);
  if (json === lastSaved) return;
  clearTimeout(timer);
  timer = setTimeout(() => void flush(), 400);
}

async function flush() {
  timer = undefined;
  const c = store.config;
  if (!c) return;
  const snap = $state.snapshot(c) as Config;
  const json = JSON.stringify(snap);
  if (json === lastSaved) return;
  store.save = 'saving';
  try {
    await api.saveConfig(snap);
    lastSaved = json;
    store.save = 'saved';
    clearTimeout(savedTimer);
    savedTimer = setTimeout(() => (store.save = 'idle'), 2400);
  } catch (e) {
    store.save = 'failed';
    store.error = String(e);
  }
}

/** Save immediately (window closing). */
export function flushNow() {
  if (timer) {
    clearTimeout(timer);
    void flush();
  }
}
