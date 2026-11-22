// IPC facade. In Tauri it talks to Rust; in a plain browser (vite dev / preview) it
// lazy-loads a mock backend that simulates sessions, streaming and errors, so the UI
// can be developed and screenshotted without the app shell.

import { invoke as tauriInvoke } from '@tauri-apps/api/core';
import { listen as tauriListen } from '@tauri-apps/api/event';
import type {
  Config,
  EngineConfig,
  EventMap,
  EventName,
  HotkeyStatus,
  Mode,
  PerfStats,
  SessionSnapshot,
  ThemeChoice,
} from './types';

export type Unlisten = () => void;

export interface Backend {
  invoke<T>(cmd: string, args?: Record<string, unknown>): Promise<T>;
  listen<K extends EventName>(event: K, cb: (payload: EventMap[K]) => void): Promise<Unlisten>;
}

export const inTauri = typeof window !== 'undefined' && '__TAURI_INTERNALS__' in window;

const tauriBackend: Backend = {
  invoke: (cmd, args) => tauriInvoke(cmd, args),
  listen: (event, cb) => tauriListen(event, (e) => cb(e.payload as never)),
};

let mock: Promise<Backend> | null = null;
function backend(): Promise<Backend> {
  if (inTauri) return Promise.resolve(tauriBackend);
  // Separate chunk: never fetched inside the app.
  return (mock ??= import('./mock').then((m) => m.createMockBackend()));
}

async function call<T>(cmd: string, args?: Record<string, unknown>): Promise<T> {
  return (await backend()).invoke<T>(cmd, args);
}

export async function on<K extends EventName>(
  event: K,
  cb: (payload: EventMap[K]) => void,
): Promise<Unlisten> {
  return (await backend()).listen(event, cb);
}

function windowLabel(): string | undefined {
  // Read without importing the (large) Window class.
  const w = window as unknown as {
    __TAURI_INTERNALS__?: { metadata?: { currentWindow?: { label?: string } } };
  };
  return w.__TAURI_INTERNALS__?.metadata?.currentWindow?.label;
}

export const api = {
  getConfig: () => call<Config>('get_config'),
  saveConfig: (config: Config) => call<void>('save_config', { config }),
  getSessionSnapshot: () => call<SessionSnapshot | null>('get_session_snapshot'),
  retranslate: (text: string, sourceLang: string, targetLang: string) =>
    call<number>('retranslate', { text, sourceLang, targetLang }),
  /** Contract change request #1. Callers fall back to `retranslate` when it is missing. */
  retranslateEngine: (sessionId: number, engineId: string, text: string, sourceLang: string, targetLang: string) =>
    call<void>('retranslate_engine', { sessionId, engineId, text, sourceLang, targetLang }),
  testEngine: (engine: EngineConfig) => call<string>('test_engine', { engine }),
  copyText: (text: string) => call<void>('copy_text', { text }),
  popupSetPinned: (pinned: boolean) => call<void>('popup_set_pinned', { pinned }),
  popupHide: () => call<void>('popup_hide'),
  popupResize: (height: number) => call<void>('popup_resize', { height }),
  startCapture: (mode: Mode) => call<void>('start_capture', { mode }),
  openSettings: () => call<void>('open_settings'),
  /** Contract change request #4. */
  checkHotkey: (accelerator: string) => call<HotkeyStatus>('check_hotkey', { accelerator }),
  /** Contract change request #5. */
  suspendHotkeys: (suspended: boolean) => call<void>('hotkeys_suspend', { suspended }),
  /** v0.2 performance data. */
  getPerfStats: () => call<PerfStats>('get_perf_stats'),
  resetPerfStats: () => call<void>('reset_perf_stats'),
  /** Plain-text report of stats and system info; the UI copies it with `copyText`. */
  getDiagnostics: () => call<string>('get_diagnostics'),
  /** Needs tauri-plugin-opener with `opener:allow-open-url`. */
  openUrl: (url: string) =>
    inTauri ? call<void>('plugin:opener|open_url', { url }) : Promise.resolve(void window.open(url, '_blank')),
  appVersion: () => (inTauri ? call<string>('plugin:app|version') : Promise.resolve('0.2.6')),
  /** Keeps the Mica material in step with a theme override. Needs `core:window:allow-set-theme`. */
  setWindowTheme: (theme: ThemeChoice) => {
    const label = windowLabel();
    if (!inTauri || !label) return Promise.resolve();
    return call<void>('plugin:window|set_theme', { label, value: theme === 'system' ? null : theme });
  },
};
