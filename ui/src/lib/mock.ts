// Browser-only mock of the Rust side. Loaded lazily by ipc.ts when __TAURI_INTERNALS__ is absent.
//
// Query parameters (popup.html):
//   ?scene=live        default: OCR after 200 ms, two engines stream, one fails
//   ?scene=recognizing | pending | streaming | done | error | ocr | notext | ocrerror
//   ?freeze=<ms>       run the timeline up to <ms> instantly and stop there
//   ?ui=en-US          English UI      ?font=16   font size      ?theme=dark|light
// settings.html accepts ?ui, ?theme and ?section=<id>.

import { defaultConfig, normalizeConfig } from './engines';
import type { Backend } from './ipc';
import type { Config, EngineConfig, EventMap, EventName, Mode, SessionSnapshot } from './types';

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
        Object.assign(tr(s.engineId), { status: 'done', text: s.text, elapsedMs: s.elapsedMs });
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

  function streamEngine(id: number, e: EngineConfig, name: string, start: number, opts: { fail?: boolean; size: number; every: number }) {
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
    at(start + total + 20, () => emit('tr:done', { sessionId: id, engineId: e.id, text, elapsedMs: total + 60 }));
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
      });
    });
  }

  function runSession(mode: Mode) {
    const id = ++sessionId;
    emit('session:start', { sessionId: id, mode, sourceLang: config.translate.sourceLang, targetLang: config.translate.targetLang });
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
        elapsedMs: mode === 'ocr' ? 64 : 86,
        engine: 'PaddleOCR',
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
      if ((e.kind === 'anthropic' || e.kind === 'youdao' || e.kind === 'baidu') && !e.apiKey) throw 'missing configuration: apiKey';
      return '你好，世界。';
    },
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
