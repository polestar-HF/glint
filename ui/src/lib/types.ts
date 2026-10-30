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
    /** v0.2: show capture / OCR / first-token timings in the popup. */
    showTimings?: boolean;
  };
  overlay: { snapWindows: boolean; magnifier: boolean };
}

export interface OcrLine {
  text: string;
  box: [number, number, number, number];
  confidence: number;
}

// ── Events ──────────────────────────────────────────────────────────────────

/** v0.2 performance data. All values in milliseconds. */
export interface CaptureTimings {
  hotkeyToOverlayMs?: number;
  captureMs?: number;
  /** End of capture → overlay visible. */
  overlayMs?: number;
}
export interface OcrTimings {
  /** Present only when the model was loaded for this capture (cold start). */
  loadMs?: number;
  detMs?: number;
  recMs?: number;
}

export interface SessionStart {
  sessionId: number;
  mode: Mode;
  sourceLang: string;
  targetLang: string;
  /** v0.2; absent for `retranslate`. */
  timings?: CaptureTimings | null;
}
export interface OcrResultEvent {
  sessionId: number;
  text: string;
  lines: OcrLine[];
  detectedLang: string | null;
  elapsedMs: number;
  engine: string;
  /** v0.2. */
  timings?: OcrTimings | null;
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
  /** v0.2: engine start → first delta; present when at least one delta arrived. */
  firstTokenMs?: number | null;
}
export interface TrError {
  sessionId: number;
  engineId: string;
  message: string;
  /** v0.2.2: error category (see lib/errors.ts). */
  code?: string | null;
  /** v0.2.2: rate-limited engines: seconds until they are asked again. */
  retryAfterSecs?: number | null;
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
  /** v0.2. */
  firstTokenMs?: number | null;
  /** v0.2.2. */
  code?: string | null;
  retryAfterSecs?: number | null;
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
  /** v0.2. */
  timings?: CaptureTimings | null;
}

// ── Performance stats (v0.2) ────────────────────────────────────────────────

export type PerfMetricKey = 'hotkeyToOverlay' | 'capture' | 'overlay' | 'ocr' | 'ocrCold' | 'ocrLoad' | 'det' | 'rec';

export interface PerfMetric {
  key: PerfMetricKey;
  n: number;
  p50: number;
  p90: number;
  max: number;
}
export interface PerfEngine {
  engineId: string;
  engineName: string;
  n: number;
  errors: number;
  p50: number;
  p90: number;
  firstTokenP50?: number | null;
}
export interface PerfStats {
  sessions: number;
  /** RFC 3339. */
  since: string;
  /** Only keys with n > 0. */
  metrics: PerfMetric[];
  engines: PerfEngine[];
  system: {
    app: string;
    os: string;
    cpuThreads: number;
    webview: string;
    monitors: Array<{ width: number; height: number; scale: number }>;
  };
}

export type HotkeyStatus = 'ok' | 'in_use' | 'invalid';
