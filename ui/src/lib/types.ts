// IPC contract types. Source of truth: docs/ARCHITECTURE.md ("IPC" and "Config").

export type Mode = 'translate' | 'ocr';
export type ThemeChoice = 'system' | 'light' | 'dark';
export type UiLanguage = 'zh-CN' | 'en-US';
export type EngineKind = 'google' | 'bing' | 'anthropic' | 'openai' | 'deepl' | 'youdao' | 'baidu';

export interface EngineConfig {
  id: string;
  kind: EngineKind | string;
  enabled: boolean;
  name?: string | null;
  apiKey?: string | null;
  apiSecret?: string | null;
  baseUrl?: string | null;
  model?: string | null;
  prompt?: string | null;
}

export interface Config {
  general: { uiLanguage: UiLanguage; autostart: boolean };
  hotkeys: { translate: string; ocr: string; silentOcr: string };
  ocr: { engine: 'paddle' | 'windows'; mergeLines: boolean; idleUnloadSecs: number };
  translate: {
    sourceLang: string;
    targetLang: string;
    secondaryLang: string;
    engines: EngineConfig[];
  };
  ui: {
    theme: ThemeChoice;
    fontSize: number;
    accent: string;
    closeOnBlur: boolean;
    popupIdleDestroySecs: number;
  };
  overlay: { snapWindows: boolean; magnifier: boolean };
}

export interface OcrLine {
  text: string;
  box: [number, number, number, number];
  confidence: number;
}

// ── Events ──────────────────────────────────────────────────────────────────

export interface SessionStart {
  sessionId: number;
  mode: Mode;
  sourceLang: string;
  targetLang: string;
}
export interface OcrResultEvent {
  sessionId: number;
  text: string;
  lines: OcrLine[];
  detectedLang: string | null;
  elapsedMs: number;
  engine: string;
}
export interface OcrErrorEvent {
  sessionId: number;
  message: string;
}
export interface TrStart {
  sessionId: number;
  engineId: string;
  engineName: string;
}
export interface TrDelta {
  sessionId: number;
  engineId: string;
  delta: string;
  /** Optional (contract change request #3): 0-based index of this delta for the engine. */
  seq?: number;
}
export interface TrDone {
  sessionId: number;
  engineId: string;
  text: string;
  elapsedMs: number;
}
export interface TrError {
  sessionId: number;
  engineId: string;
  message: string;
}

export interface EventMap {
  'session:start': SessionStart;
  'ocr:result': OcrResultEvent;
  'ocr:error': OcrErrorEvent;
  'tr:start': TrStart;
  'tr:delta': TrDelta;
  'tr:done': TrDone;
  'tr:error': TrError;
  'config:changed': Config;
}
export type EventName = keyof EventMap;

// ── Snapshot ────────────────────────────────────────────────────────────────

export type TrStatus = 'pending' | 'streaming' | 'done' | 'error';

export interface SnapshotTranslation {
  engineId: string;
  engineName: string;
  status: TrStatus;
  text: string;
  message?: string | null;
  elapsedMs?: number | null;
  /** Optional (contract change request #3): number of deltas already folded into `text`. */
  deltaCount?: number;
}

export interface SessionSnapshot {
  sessionId: number;
  mode: Mode;
  sourceLang: string;
  targetLang: string;
  ocr: {
    status: 'pending' | 'done' | 'error';
    result?: Omit<OcrResultEvent, 'sessionId'> | null;
    message?: string | null;
  };
  translations: SnapshotTranslation[];
}

export type HotkeyStatus = 'ok' | 'in_use' | 'invalid';
