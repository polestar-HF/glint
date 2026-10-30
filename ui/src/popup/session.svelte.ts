// Popup state: one translation session at a time. Events for older sessions are dropped.

import type {
  CaptureTimings,
  Mode,
  OcrTimings,
  OcrErrorEvent,
  OcrResultEvent,
  SessionSnapshot,
  SessionStart,
  TrDelta,
  TrDone,
  TrError,
  TrStart,
} from '../lib/types';

export type ResultStatus = 'waiting' | 'pending' | 'streaming' | 'done' | 'error';

export interface ResultView {
  engineId: string;
  engineName: string;
  status: ResultStatus;
  text: string;
  message: string;
  elapsedMs: number | null;
  collapsed: boolean;
  /** Deltas applied so far (for de-duplicating replays after a snapshot). */
  deltas: number;
  /** v0.2: engine start → first delta. */
  firstTokenMs: number | null;
  /** v0.2.2: error category and rate-limit wait (see lib/errors.ts). */
  code: string | null;
  retryAfterSecs: number | null;
}

export interface PredictedEngine {
  id: string;
  name: string;
}

export class Session {
  id = $state(0);
  mode = $state<Mode>('translate');
  sourceLang = $state('auto');
  targetLang = $state('zh-Hans');

  ocrStatus = $state<'pending' | 'done' | 'error'>('pending');
  ocrEngine = $state('');
  ocrElapsed = $state<number | null>(null);
  ocrMessage = $state('');
  detectedLang = $state<string | null>(null);
  lineCount = $state(0);
  /** Text as OCR returned it. */
  ocrText = $state('');
  /** Text the current translations were made from. */
  submittedText = $state('');
  /** Editable source. */
  text = $state('');

  results = $state<ResultView[]>([]);
  sourceCollapsed = $state(false);

  /** v0.2 timings. Kept across a `retranslate`: the capture and OCR they describe are unchanged. */
  captureTimings = $state<CaptureTimings | null>(null);
  ocrTimings = $state<OcrTimings | null>(null);

  /** Set while a `retranslate` call is in flight: the next session keeps the edited text. */
  expectRetranslate: { text: string } | null = null;
  /** Language changed while OCR was still running; translate again once text arrives. */
  langChangedEarly = false;

  dirty = $derived(this.ocrStatus === 'done' && this.text.trim() !== this.submittedText.trim());
  edited = $derived(this.ocrStatus === 'done' && this.submittedText !== this.ocrText);
  primary = $derived(this.results.find((r) => r.text && (r.status === 'done' || r.status === 'streaming')));

  constructor(private predict: () => PredictedEngine[]) {}

  /** Returns false for events that belong to an older session. */
  private gate(sessionId: number): boolean {
    if (sessionId < this.id) return false;
    if (sessionId > this.id) {
      this.start({ sessionId, mode: this.mode, sourceLang: this.sourceLang, targetLang: this.targetLang });
    }
    return true;
  }

  private placeholders(): ResultView[] {
    return this.predict().map((e) => blank(e.id, e.name, 'waiting'));
  }

  start(p: SessionStart) {
    if (p.sessionId < this.id) return;
    if (p.sessionId === this.id) {
      // Duplicate start (already applied from a snapshot, or the backend reporting the
      // language pair it really uses once OCR has detected the text language): only
      // refresh langs/mode.
      this.mode = p.mode;
      this.sourceLang = p.sourceLang;
      this.targetLang = p.targetLang;
      if (p.timings) this.captureTimings = p.timings;
      return;
    }
    this.id = p.sessionId;
    this.mode = p.mode;
    this.sourceLang = p.sourceLang;
    this.targetLang = p.targetLang;
    this.langChangedEarly = false;
    const re = this.expectRetranslate;
    this.expectRetranslate = null;
    if (re) {
      // Same source, new translations. OCR metadata stays as it was.
      this.ocrStatus = 'done';
      this.submittedText = re.text;
      this.text = re.text;
    } else {
      this.captureTimings = p.timings ?? null;
      this.ocrTimings = null;
      this.ocrStatus = 'pending';
      this.ocrEngine = '';
      this.ocrElapsed = null;
      this.ocrMessage = '';
      this.detectedLang = null;
      this.lineCount = 0;
      this.ocrText = '';
      this.submittedText = '';
      this.text = '';
      this.sourceCollapsed = false;
    }
    const keepCollapsed = new Map(this.results.map((r) => [r.engineId, r.collapsed]));
    this.results = p.mode === 'translate' ? this.placeholders() : [];
    for (const r of this.results) r.collapsed = keepCollapsed.get(r.engineId) ?? false;
  }

  ocrResult(p: OcrResultEvent) {
    if (!this.gate(p.sessionId)) return;
    this.ocrStatus = 'done';
    this.ocrEngine = p.engine;
    this.ocrElapsed = p.elapsedMs;
    this.ocrTimings = p.timings ?? null;
    // OCR reports a bare "zh" (it cannot tell scripts apart); the UI's language list has no such
    // code, so name it, and compare it with the target, as Simplified.
    this.detectedLang = p.detectedLang === 'zh' ? 'zh-Hans' : p.detectedLang;
    this.lineCount = p.lines?.length ?? 0;
    this.ocrText = p.text;
    this.submittedText = p.text;
    this.text = p.text;
    if (!p.text.trim()) this.results = [];
  }

  ocrError(p: OcrErrorEvent) {
    if (!this.gate(p.sessionId)) return;
    this.ocrStatus = 'error';
    this.ocrMessage = p.message;
    this.results = [];
  }

  private result(engineId: string, engineName?: string): ResultView {
    let r = this.results.find((x) => x.engineId === engineId);
    if (!r) {
      this.results.push(blank(engineId, engineName ?? engineId, 'pending'));
      r = this.results[this.results.length - 1];
    }
    return r;
  }

  trStart(p: TrStart) {
    if (!this.gate(p.sessionId)) return;
    // Keep the localized / user-renamed name from config when we already have one.
    const r = this.result(p.engineId, p.engineName);
    r.engineName = r.engineName || p.engineName;
    // A replayed start must not wipe text that is already streaming in.
    if (r.status === 'streaming' || r.status === 'done') return;
    r.status = 'pending';
    r.text = '';
    r.message = '';
    r.elapsedMs = null;
    r.deltas = 0;
    r.firstTokenMs = null;
    r.code = null;
    r.retryAfterSecs = null;
  }

  trDelta(p: TrDelta) {
    if (!this.gate(p.sessionId)) return;
    const r = this.result(p.engineId);
    if (r.status === 'done' || r.status === 'error') return;
    if (p.seq != null && p.seq < r.deltas) return; // already folded in (snapshot replay)
    r.text += p.delta;
    r.deltas += 1;
    r.status = 'streaming';
  }

  trDone(p: TrDone) {
    if (!this.gate(p.sessionId)) return;
    const r = this.result(p.engineId);
    r.text = p.text;
    r.status = 'done';
    r.elapsedMs = p.elapsedMs;
    r.firstTokenMs = p.firstTokenMs ?? null;
  }

  trError(p: TrError) {
    if (!this.gate(p.sessionId)) return;
    const r = this.result(p.engineId);
    r.status = 'error';
    r.message = p.message;
    r.code = p.code ?? null;
    r.retryAfterSecs = p.retryAfterSecs ?? null;
  }

  /** Drops placeholders for engines that never started (config changed under us). */
  prune() {
    if (this.results.some((r) => r.status !== 'waiting')) {
      this.results = this.results.filter((r) => r.status !== 'waiting');
    }
  }

  fromSnapshot(s: SessionSnapshot) {
    if (s.sessionId < this.id) return;
    this.id = 0; // force a fresh start for this id
    this.expectRetranslate = null;
    this.start({
      sessionId: s.sessionId,
      mode: s.mode,
      sourceLang: s.sourceLang,
      targetLang: s.targetLang,
      timings: s.timings,
    });
    if (s.ocr.status === 'done' && s.ocr.result) {
      this.ocrResult({ sessionId: s.sessionId, ...s.ocr.result });
    } else if (s.ocr.status === 'error') {
      this.ocrError({ sessionId: s.sessionId, message: s.ocr.message ?? '' });
    }
    if (s.mode === 'translate' && s.translations.length) {
      this.results = s.translations.map((t) => ({
        ...blank(t.engineId, t.engineName, t.status === 'pending' ? 'pending' : t.status),
        text: t.text ?? '',
        message: t.message ?? '',
        elapsedMs: t.elapsedMs ?? null,
        deltas: t.deltaCount ?? 0,
        firstTokenMs: t.firstTokenMs ?? null,
        code: t.code ?? null,
        retryAfterSecs: t.retryAfterSecs ?? null,
      }));
    }
  }
}

function blank(engineId: string, engineName: string, status: ResultStatus): ResultView {
  return {
    engineId,
    engineName,
    status,
    text: '',
    message: '',
    elapsedMs: null,
    collapsed: false,
    deltas: 0,
    firstTokenMs: null,
    code: null,
    retryAfterSecs: null,
  };
}
