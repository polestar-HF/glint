// Browser-only mock of the Rust side. Loaded lazily by ipc.ts when __TAURI_INTERNALS__ is absent.
//
// Query parameters (popup.html):
//   ?scene=live        default: OCR after 200 ms, two engines stream, one fails
//   ?scene=recognizing | pending | streaming | done | error | ocr | notext | ocrerror
//   ?freeze=<ms>       run the timeline up to <ms> instantly and stop there
//   ?ui=en-US          English UI      ?font=16   font size      ?theme=dark|light
//   ?timings=1         ui.showTimings on        ?cold=1    OCR includes a model load
// settings.html accepts ?ui, ?theme, ?section=<id>, ?engines=multi (two user-added LLM
// services, one missing its model) and ?perf=empty (no statistics yet).

import { defaultConfig, normalizeConfig } from './engines';
import type { Backend } from './ipc';
import type { Config, EngineConfig, EventMap, EventName, Mode, PerfStats, SessionSnapshot } from './types';

const q = new URLSearchParams(location.search);
const scene = q.get('scene') ?? 'live';

const SOURCE_EN =
  'Screen readers announce a control’s accessible name first, then its role and state. ' +
  'When the name is missing, people hear only the role (“button, button, button”) and the interface becomes hard to navigate.';
const OUT: Record<string, string> = {
  claude:
    '屏幕阅读器会先读出控件的可访问名称，再读出它的角色和状态。如果缺少名称，用户只能听到角色（“按钮、按钮、按钮”），界面就很难操作。',
  google:
    '屏幕阅读器首先宣布控件的可访问名称，然后宣布其角色和状态。当名称缺失时，人们只能听到角色（“按钮，按钮，按钮”），界面变得难以导航。',
  bing: '屏幕阅读器会首先播报控件的无障碍名称，然后是其角色和状态。缺少名称时，用户只会听到角色（“按钮，按钮，按钮”），界面就难以浏览。',
};
const SOURCE_ZH =
  '本周五 18:00 起，主站将进行例行维护，预计持续两小时。\n' +
  '维护期间无法登录和提交订单，已提交的订单不受影响。\n' +
  '如有疑问，请联系在线客服。';

function mockConfig(): Config {
  const c = defaultConfig();
  const byId = (id: string) => c.translate.engines.find((e) => e.id === id)!;
  const claude = { ...byId('claude'), enabled: true, apiKey: 'sk-ant-api03-••••••••••••' };
  const deepl = { ...byId('deepl'), enabled: scene !== 'done' };
  const google = byId('google');
  const bing = { ...byId('bing'), enabled: false };
  c.translate.engines = [claude, google, deepl, bing, byId('openai'), byId('youdao'), byId('baidu')];
  if (q.get('engines') === 'multi') {
    // Two services added from presets; 通义千问 still has no model, so it shows a hint.
    const deepseek: EngineConfig = {
      id: 'openai-k3x9q2',
      kind: 'openai',
      enabled: true,
      name: 'DeepSeek',
      baseUrl: 'https://api.deepseek.com/v1',
      model: 'deepseek-chat',
      apiKey: 'sk-••••••••••••••••',
    };
    const qwen: EngineConfig = {
      id: 'openai-7mf2ad',
      kind: 'openai',
      enabled: true,
      name: '通义千问',
      baseUrl: 'https://dashscope.aliyuncs.com/compatible-mode/v1',
      model: null,
      apiKey: 'sk-••••••••••••••••',
    };
    c.translate.engines = [claude, google, deepseek, qwen, deepl, bing, byId('openai'), byId('youdao'), byId('baidu')];
  }
  if (q.get('timings') === '1') c.ui.showTimings = true;
  if (q.get('ui') === 'en-US') c.general.uiLanguage = 'en-US';
  const font = Number(q.get('font'));
  if (font) c.ui.fontSize = font;
  const theme = q.get('theme');
  if (theme === 'light' || theme === 'dark') c.ui.theme = theme;
  return c;
}

type Listener = (payload: unknown) => void;

export function createMockBackend(): Backend {
  const listeners = new Map<string, Set<Listener>>();
  let config = normalizeConfig(mockConfig());
  let snapshot: SessionSnapshot | null = null;
  let sessionId = 0;
  let clock = 0; // virtual time while replaying a frozen scene
  const freezeAt = q.has('freeze') ? Number(q.get('freeze')) : freezeFor(scene);
  let frozen = freezeAt != null;
  const timers: Array<{ at: number; fn: () => void }> = [];
  const cold = q.get('cold') === '1';
  const coldLoad = cold ? { loadMs: 1046 } : {};
  let perf: PerfStats = q.get('perf') === 'empty' ? emptyPerf() : samplePerf();

  function emit<K extends EventName>(event: K, payload: EventMap[K]) {
    track(event, payload);
    listeners.get(event)?.forEach((cb) => cb(structuredClone(payload)));
  }

  /** Mirrors what the Rust side would keep for get_session_snapshot. */
  function track<K extends EventName>(event: K, p: EventMap[K]) {
    const any = p as unknown as Record<string, unknown>;
    if (event === 'session:start') {
      const s = p as EventMap['session:start'];
      snapshot = { ...s, ocr: { status: 'pending' }, translations: [] };
      return;
    }
    if (!snapshot || any.sessionId !== snapshot.sessionId) return;
    const tr = (id: string) => snapshot!.translations.find((t) => t.engineId === id)!;
    switch (event) {
      case 'ocr:result': {
        const { sessionId: _, ...result } = p as EventMap['ocr:result'];
        snapshot.ocr = { status: 'done', result };
        break;
      }
      case 'ocr:error':
        snapshot.ocr = { status: 'error', message: (p as EventMap['ocr:error']).message };
        break;
      case 'tr:start': {
        const s = p as EventMap['tr:start'];
        snapshot.translations = snapshot.translations.filter((t) => t.engineId !== s.engineId);
        snapshot.translations.push({ engineId: s.engineId, engineName: s.engineName, status: 'pending', text: '', deltaCount: 0 });
        break;
      }
      case 'tr:delta': {
        const s = p as EventMap['tr:delta'];
        const t = tr(s.engineId);
        t.text += s.delta;
        t.status = 'streaming';
        t.deltaCount = (t.deltaCount ?? 0) + 1;
        break;
      }
      case 'tr:done': {
        const s = p as EventMap['tr:done'];
        Object.assign(tr(s.engineId), { status: 'done', text: s.text, elapsedMs: s.elapsedMs, firstTokenMs: s.firstTokenMs });
        break;
      }
      case 'tr:error': {
        const s = p as EventMap['tr:error'];
        Object.assign(tr(s.engineId), { status: 'error', message: s.message });
        break;
      }
    }
  }

  function at(ms: number, fn: () => void) {
    if (frozen) {
      const when = clock + ms;
      if (when <= freezeAt!) timers.push({ at: when, fn });
      return;
    }
    setTimeout(fn, ms);
  }

  function runFrozen() {
    // Deterministic: execute everything up to the freeze point in time order.
    while (timers.length) {
      timers.sort((a, b) => a.at - b.at);
      const next = timers.shift()!;
      clock = next.at;
      next.fn();
    }
  }

  function chunks(text: string, size: number): string[] {
    const out: string[] = [];
    for (let i = 0; i < text.length; i += size) out.push(text.slice(i, i + size));
    return out;
  }

  function streamEngine(
    id: number,
    e: EngineConfig,
    name: string,
    start: number,
    opts: { fail?: boolean; size: number; every: number; fast?: boolean },
  ) {
    at(start, () => emit('tr:start', { sessionId: id, engineId: e.id, engineName: name }));
    if (opts.fail) {
      at(start + 1300, () =>
        emit('tr:error', { sessionId: id, engineId: e.id, message: 'service error (456): Quota exceeded for this billing period' }),
      );
      return;
    }
    const text = OUT[e.id] ?? OUT.bing;
    const parts = chunks(text, opts.size);
    let seq = 0;
    parts.forEach((delta, i) =>
      at(start + 80 + i * opts.every, () => emit('tr:delta', { sessionId: id, engineId: e.id, delta, seq: seq++ })),
    );
    const total = 80 + parts.length * opts.every;
    const elapsedMs = total + 60;
    // Web endpoints answer in one go (first token ≈ the whole request); LLMs stream early.
    const firstTokenMs = Math.round(elapsedMs * (opts.fast ? 0.82 : 0.55));
    at(start + total + 20, () => emit('tr:done', { sessionId: id, engineId: e.id, text, elapsedMs, firstTokenMs }));
  }

  const names: Record<string, string> = { claude: 'Claude', google: 'Google 翻译', deepl: 'DeepL', bing: '必应翻译' };
  const nameOf = (e: EngineConfig) => e.name || names[e.id] || e.kind;

  function translateAll(id: number, start: number) {
    const enabled = config.translate.engines.filter((e) => e.enabled);
    enabled.forEach((e, i) => {
      const fast = e.kind === 'google' || e.kind === 'bing';
      streamEngine(id, e, nameOf(e), start + i * 6, {
        fail: e.kind === 'deepl',
        size: fast ? 12 : 3,
        every: fast ? 70 : 42,
        fast,
      });
    });
  }

  function runSession(mode: Mode) {
    const id = ++sessionId;
    emit('session:start', {
      sessionId: id,
      mode,
      sourceLang: config.translate.sourceLang,
      targetLang: config.translate.targetLang,
      timings: { hotkeyToOverlayMs: 58, captureMs: 21, overlayMs: 34 },
    });
    if (scene === 'recognizing') return;
    at(200, () => {
      if (scene === 'ocrerror') {
        emit('ocr:error', { sessionId: id, message: 'ONNX Runtime: failed to load model (det.onnx)' });
        return;
      }
      const text = scene === 'notext' ? '' : mode === 'ocr' ? SOURCE_ZH : SOURCE_EN;
      emit('ocr:result', {
        sessionId: id,
        text,
        lines: text.split('\n').filter(Boolean).map((t, i) => ({ text: t, box: [0, i * 24, 400, 20], confidence: 0.97 })),
        detectedLang: text ? (mode === 'ocr' ? 'zh-Hans' : 'en') : null,
        // det + rec + pre/post-processing; a cold start adds the model load.
        elapsedMs: (mode === 'ocr' ? 118 : 142) + (cold ? 1046 : 0),
        engine: 'PaddleOCR',
        timings: mode === 'ocr' ? { detMs: 41, recMs: 66, ...coldLoad } : { detMs: 48, recMs: 81, ...coldLoad },
      });
      if (mode === 'translate' && text) translateAll(id, 40);
    });
  }

  // Kick off the scene as soon as the backend exists; the popup catches up via snapshot.
  if (location.pathname.includes('popup')) {
    runSession(scene === 'ocr' ? 'ocr' : 'translate');
    if (frozen) runFrozen();
  }
  // Anything triggered by interaction afterwards runs in real time.
  frozen = false;

  const commands: Record<string, (args: Record<string, unknown>) => unknown> = {
    get_config: () => structuredClone(config),
    save_config: ({ config: c }) => {
      config = normalizeConfig(c as Config);
      emit('config:changed', structuredClone(config));
    },
    get_session_snapshot: () => (snapshot ? structuredClone(snapshot) : null),
    retranslate: ({ sourceLang, targetLang }) => {
      const id = ++sessionId;
      setTimeout(() => {
        emit('session:start', { sessionId: id, mode: 'translate', sourceLang: String(sourceLang), targetLang: String(targetLang) });
        translateAll(id, 20);
      }, 10);
      return id;
    },
    retranslate_engine: ({ sessionId: sid, engineId }) => {
      const e = config.translate.engines.find((x) => x.id === engineId);
      if (!e) throw new Error('unknown engine');
      // The retry succeeds, so the preview shows the recovered state.
      setTimeout(() => streamEngine(Number(sid), e, nameOf(e), 0, { size: 6, every: 50 }), 10);
      return null;
    },
    test_engine: async ({ engine }) => {
      const e = engine as EngineConfig;
      await sleep(e.kind === 'google' ? 380 : 720);
      if (e.kind === 'deepl') throw 'service error (403): Authorization failed. Check the API key.';
      if ((e.kind === 'youdao' || e.kind === 'baidu') && !e.apiKey) throw 'missing configuration: apiKey';
      if (e.kind === 'anthropic' && !e.apiKey && !e.baseUrl) throw 'missing configuration: apiKey';
      if (e.kind === 'openai') {
        const official = !e.baseUrl || e.baseUrl.includes('api.openai.com');
        if (official && !e.apiKey) throw 'missing configuration: The OpenAI API key is not set';
        if (!official && !e.model) throw 'missing configuration: Set the model name for this service';
      }
      return '你好，世界。';
    },
    get_perf_stats: () => structuredClone(perf),
    reset_perf_stats: () => {
      perf = emptyPerf();
    },
    get_diagnostics: () => diagnostics(perf),
    copy_text: async ({ text }) => {
      try {
        await navigator.clipboard?.writeText(String(text));
      } catch {
        /* headless or no permission */
      }
    },
    popup_set_pinned: () => null,
    popup_hide: () => {
      document.documentElement.classList.add('preview-hidden');
      setTimeout(() => document.documentElement.classList.remove('preview-hidden'), 900);
    },
    popup_resize: ({ height }) => {
      document.documentElement.style.setProperty('--preview-h', `${Math.round(Number(height))}px`);
    },
    start_capture: ({ mode }) => runSession(mode as Mode),
    open_settings: () => void window.open('settings.html', '_blank'),
    check_hotkey: ({ accelerator }) => (/^ctrl\+alt\+a$/i.test(String(accelerator)) ? 'in_use' : 'ok'),
    hotkeys_suspend: () => null,
  };

  return {
    async invoke<T>(cmd: string, args: Record<string, unknown> = {}): Promise<T> {
      const fn = commands[cmd];
      if (!fn) throw new Error(`mock: unknown command ${cmd}`);
      return (await fn(args)) as T;
    },
    async listen(event, cb) {
      let set = listeners.get(event);
      if (!set) listeners.set(event, (set = new Set()));
      set.add(cb as Listener);
      return () => set!.delete(cb as Listener);
    },
  };
}

function freezeFor(s: string): number | null {
  switch (s) {
    case 'recognizing':
      return 100;
    case 'pending':
      return 262;
    case 'streaming':
      return 640;
    case 'live':
      return null;
    default:
      return 60_000; // run to completion instantly
  }
}

function sleep(ms: number) {
  return new Promise((r) => setTimeout(r, ms));
}

function system(): PerfStats['system'] {
  return {
    app: 'Glint 0.2.0',
    os: 'Windows 11 Pro 24H2 (26100.2314)',
    cpuThreads: 16,
    webview: '131.0.2903.70',
    monitors: [
      { width: 2560, height: 1440, scale: 1.5 },
      { width: 1920, height: 1080, scale: 1 },
    ],
  };
}

function emptyPerf(): PerfStats {
  return { sessions: 0, since: new Date().toISOString(), metrics: [], engines: [], system: system() };
}

/** Plausible numbers for a mid-range laptop after about a week of use. */
function samplePerf(): PerfStats {
  return {
    sessions: 128,
    since: '2026-10-21T09:12:00+08:00',
    metrics: [
      { key: 'hotkeyToOverlay', n: 128, p50: 58, p90: 83, max: 161 },
      { key: 'capture', n: 128, p50: 21, p90: 29, max: 64 },
      { key: 'overlay', n: 128, p50: 34, p90: 47, max: 92 },
      { key: 'ocr', n: 124, p50: 142, p90: 238, max: 611 },
      { key: 'ocrCold', n: 4, p50: 1188, p90: 1402, max: 1460 },
      { key: 'ocrLoad', n: 4, p50: 1046, p90: 1219, max: 1254 },
      { key: 'det', n: 128, p50: 48, p90: 77, max: 190 },
      { key: 'rec', n: 128, p50: 81, p90: 152, max: 402 },
    ],
    engines: [
      { engineId: 'claude', engineName: 'Claude', n: 96, errors: 2, p50: 1064, p90: 1873, firstTokenP50: 585 },
      { engineId: 'google', engineName: 'Google 翻译', n: 121, errors: 0, p50: 412, p90: 690, firstTokenP50: 338 },
      { engineId: 'openai-k3x9q2', engineName: 'DeepSeek', n: 22, errors: 1, p50: 1640, p90: 2904, firstTokenP50: 893 },
      { engineId: 'deepl', engineName: 'DeepL', n: 37, errors: 9, p50: 520, p90: 884, firstTokenP50: null },
    ],
    system: system(),
  };
}

function diagnostics(p: PerfStats): string {
  const pad = (s: string | number, n: number) => String(s).padStart(n);
  const lines = [
    `${p.system.app} diagnostics`,
    `sessions: ${p.sessions} since ${p.since}`,
    '',
    `os: ${p.system.os}`,
    `cpu threads: ${p.system.cpuThreads}`,
    `webview2: ${p.system.webview}`,
    `monitors: ${p.system.monitors.map((m) => `${m.width}x${m.height}@${m.scale}`).join(', ')}`,
    '',
    `metric (ms)        ${pad('n', 5)}${pad('p50', 7)}${pad('p90', 7)}${pad('max', 7)}`,
    ...p.metrics.map((m) => `${m.key.padEnd(19)}${pad(m.n, 5)}${pad(m.p50, 7)}${pad(m.p90, 7)}${pad(m.max, 7)}`),
    '',
    `engine (ms)        ${pad('n', 5)}${pad('err', 5)}${pad('p50', 7)}${pad('p90', 7)}${pad('first', 7)}`,
    ...p.engines.map(
      (e) => `${e.engineId.padEnd(19)}${pad(e.n, 5)}${pad(e.errors, 5)}${pad(e.p50, 7)}${pad(e.p90, 7)}${pad(e.firstTokenP50 ?? '-', 7)}`,
    ),
  ];
  return lines.join('\n');
}
