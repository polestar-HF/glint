import { t, type MessageKey } from './i18n.svelte';
import type { Config, EngineConfig } from './types';

export type FieldKey = 'name' | 'apiKey' | 'apiSecret' | 'baseUrl' | 'model' | 'prompt';

export interface FieldSpec {
  key: FieldKey;
  label: MessageKey;
  secret?: boolean;
  optional?: boolean;
  multiline?: boolean;
  placeholder?: string;
  hint?: MessageKey;
}

interface KindSpec {
  free: boolean;
  /** Fields that must be filled before the engine can work. */
  required: FieldKey[];
  fields: FieldSpec[];
}

const prompt: FieldSpec = { key: 'prompt', label: 'field.prompt', optional: true, multiline: true };

export const KINDS: Record<string, KindSpec> = {
  google: { free: true, required: [], fields: [] },
  bing: { free: true, required: [], fields: [] },
  anthropic: {
    free: false,
    // Conditional (see missingFields): key unless a base URL points at a proxy.
    required: [],
    fields: [
      { key: 'apiKey', label: 'field.apiKey', secret: true, placeholder: 'sk-ant-…', hint: 'hint.anthropicKey' },
      { key: 'model', label: 'field.model', placeholder: 'claude-haiku-5-5' },
      { key: 'baseUrl', label: 'field.baseUrl', optional: true, placeholder: 'https://api.anthropic.com' },
      prompt,
    ],
  },
  openai: {
    free: false,
    // Conditional (see missingFields): key for official endpoints, model for custom ones.
    required: [],
    fields: [
      { key: 'baseUrl', label: 'field.baseUrl', placeholder: 'https://api.openai.com/v1', hint: 'hint.openaiBase' },
      { key: 'apiKey', label: 'field.apiKey', secret: true, optional: true, placeholder: 'sk-…' },
      { key: 'model', label: 'field.model', placeholder: 'gpt-5-mini' },
      prompt,
    ],
  },
  deepl: {
    free: false,
    required: ['apiKey'],
    fields: [{ key: 'apiKey', label: 'field.apiKey', secret: true, hint: 'hint.deeplKey' }],
  },
  youdao: {
    free: false,
    required: ['apiKey', 'apiSecret'],
    fields: [
      { key: 'apiKey', label: 'field.appId' },
      { key: 'apiSecret', label: 'field.appSecret', secret: true },
    ],
  },
  baidu: {
    free: false,
    required: ['apiKey', 'apiSecret'],
    fields: [
      { key: 'apiKey', label: 'field.appId' },
      { key: 'apiSecret', label: 'field.secretKey', secret: true },
    ],
  },
};

export function kindSpec(kind: string): KindSpec {
  return KINDS[kind] ?? { free: false, required: [], fields: [prompt] };
}

export function engineName(e: Pick<EngineConfig, 'kind' | 'name'>): string {
  if (e.name && e.name.trim()) return e.name.trim();
  const key = `engine.${e.kind}` as MessageKey;
  const s = t(key);
  return s === key ? e.kind : s;
}

// ── Built-in vs user-added LLM services (v0.2) ────────────────────────────────

/** Built-in entries can't be deleted; anything else was added by the user. */
export const BUILTIN_IDS: ReadonlySet<string> = new Set(['google', 'bing', 'claude', 'openai', 'deepl', 'youdao', 'baidu']);

export function isUserAdded(e: Pick<EngineConfig, 'id'>): boolean {
  return !BUILTIN_IDS.has(e.id);
}

export interface Preset {
  id: string;
  kind: 'openai' | 'anthropic';
  /** Brand names stay as they are; generic ones are translated. */
  label: string | { key: MessageKey };
  baseUrl: string;
  model: string;
  key: 'required' | 'none' | 'optional';
  /** "自定义": the user still has to say where the service is. */
  custom?: boolean;
}

export const PRESETS: readonly Preset[] = [
  { id: 'deepseek', kind: 'openai', label: 'DeepSeek', baseUrl: 'https://api.deepseek.com/v1', model: 'deepseek-chat', key: 'required' },
  { id: 'qwen', kind: 'openai', label: { key: 'preset.qwen' }, baseUrl: 'https://dashscope.aliyuncs.com/compatible-mode/v1', model: '', key: 'required' },
  { id: 'kimi', kind: 'openai', label: 'Kimi', baseUrl: 'https://api.moonshot.cn/v1', model: '', key: 'required' },
  { id: 'glm', kind: 'openai', label: { key: 'preset.glm' }, baseUrl: 'https://open.bigmodel.cn/api/paas/v4', model: '', key: 'required' },
  { id: 'siliconflow', kind: 'openai', label: { key: 'preset.siliconflow' }, baseUrl: 'https://api.siliconflow.cn/v1', model: '', key: 'required' },
  { id: 'openrouter', kind: 'openai', label: 'OpenRouter', baseUrl: 'https://openrouter.ai/api/v1', model: '', key: 'required' },
  { id: 'ollama', kind: 'openai', label: 'Ollama', baseUrl: 'http://localhost:11434/v1', model: '', key: 'none' },
  { id: 'lmstudio', kind: 'openai', label: 'LM Studio', baseUrl: 'http://localhost:1234/v1', model: '', key: 'none' },
  { id: 'custom-openai', kind: 'openai', label: { key: 'preset.custom' }, baseUrl: '', model: '', key: 'optional', custom: true },
  { id: 'claude', kind: 'anthropic', label: 'Claude', baseUrl: '', model: 'claude-haiku-5-5', key: 'required' },
  { id: 'custom-anthropic', kind: 'anthropic', label: { key: 'preset.customAnthropic' }, baseUrl: '', model: '', key: 'optional', custom: true },
];

export function presetLabel(p: Preset): string {
  return typeof p.label === 'string' ? p.label : t(p.label.key);
}

/** "https://api.deepseek.com/v1" → "api.deepseek.com"; keeps the port ("localhost:11434"). */
export function hostOf(url: string | null | undefined): string {
  if (!url || !url.trim()) return '';
  try {
    return new URL(url.trim()).host;
  } catch {
    return url.trim().replace(/^[a-z]+:\/\//i, '').replace(/\/.*$/, '');
  }
}

export function presetForUrl(url: string): Preset | undefined {
  const h = hostOf(url);
  return h ? PRESETS.find((p) => p.baseUrl && hostOf(p.baseUrl) === h) : undefined;
}

export function protocolLabel(kind: string): string {
  return kind === 'anthropic' ? t('tr.protoAnthropic') : t('tr.protoOpenai');
}

/** Unique id for a user-added entry: `<kind>-<6 × [a-z0-9]>`. */
export function newEngineId(kind: string, taken: Iterable<string>): string {
  const used = new Set(taken);
  const abc = 'abcdefghijklmnopqrstuvwxyz0123456789';
  for (;;) {
    const bytes = crypto.getRandomValues(new Uint8Array(6));
    const id = `${kind}-${Array.from(bytes, (b) => abc[b % abc.length]).join('')}`;
    if (!used.has(id)) return id;
  }
}

export function engineFromPreset(p: Preset, taken: Iterable<string>): EngineConfig {
  return {
    id: newEngineId(p.kind, taken),
    kind: p.kind,
    enabled: true,
    name: presetLabel(p),
    baseUrl: p.baseUrl || null,
    model: p.model || null,
    apiKey: null,
  };
}

/**
 * Required fields that are still empty, mirroring the backend's rules (v0.2 contract):
 * - openai: key on the official endpoint (no base URL, or api.openai.com) and on hosted
 *   services that need one; a model whenever the base URL is custom.
 * - anthropic: key unless a base URL is set (proxies may not need one).
 */
export function requiredFields(e: EngineConfig): FieldKey[] {
  const has = (k: FieldKey) => !!(e[k] && String(e[k]).trim());
  if (e.kind === 'openai') {
    const base = has('baseUrl') ? String(e.baseUrl).trim() : '';
    const official = !base || base.includes('api.openai.com');
    const out: FieldKey[] = [];
    if (official || presetForUrl(base)?.key === 'required') out.push('apiKey');
    if (!official) out.push('model');
    return out;
  }
  if (e.kind === 'anthropic') return has('baseUrl') ? [] : ['apiKey'];
  return kindSpec(e.kind).required;
}

export function missingFields(e: EngineConfig): FieldKey[] {
  return requiredFields(e).filter((k) => !(e[k] && String(e[k]).trim()));
}

export function isConfigured(e: EngineConfig): boolean {
  return missingFields(e).length === 0;
}

/** The inline hint for an engine that can't run yet, or '' when it can. */
export function missingHint(e: EngineConfig): string {
  const m = missingFields(e);
  const key = m.includes('apiKey') || m.includes('apiSecret');
  const model = m.includes('model');
  if (key && model) return t('tr.needsKeyModel');
  if (model) return t('tr.needsModel');
  if (key || m.length) return t('tr.needsKey');
  return '';
}

export function defaultConfig(): Config {
  const eng = (id: string, kind: string, enabled: boolean, extra: Partial<EngineConfig> = {}): EngineConfig => ({
    id,
    kind,
    enabled,
    ...extra,
  });
  return {
    general: { uiLanguage: 'zh-CN', autostart: false },
    hotkeys: { translate: 'Alt+Q', ocr: 'Alt+W', silentOcr: 'Alt+Shift+W' },
    ocr: { engine: 'paddle', mergeLines: true, idleUnloadSecs: 300 },
    translate: {
      sourceLang: 'auto',
      targetLang: 'zh-Hans',
      secondaryLang: 'en',
      engines: [
        eng('google', 'google', true),
        eng('bing', 'bing', true),
        eng('claude', 'anthropic', false, { model: 'claude-haiku-5-5' }),
        eng('openai', 'openai', false),
        eng('deepl', 'deepl', false),
        eng('youdao', 'youdao', false),
        eng('baidu', 'baidu', false),
      ],
    },
    ui: { theme: 'system', fontSize: 14, accent: '#1E6AC5', closeOnBlur: false, popupIdleDestroySecs: 600, showTimings: false },
    overlay: { snapWindows: true, magnifier: true },
  };
}

/** Fills missing keys from defaults (the Rust side does the same; this guards the UI). */
export function normalizeConfig(c: Partial<Config> | null | undefined): Config {
  const d = defaultConfig();
  if (!c) return d;
  return {
    general: { ...d.general, ...c.general },
    hotkeys: { ...d.hotkeys, ...c.hotkeys },
    ocr: { ...d.ocr, ...c.ocr },
    translate: {
      ...d.translate,
      ...c.translate,
      engines: c.translate?.engines?.length ? c.translate.engines : d.translate.engines,
    },
    ui: { ...d.ui, ...c.ui },
    overlay: { ...d.overlay, ...c.overlay },
  };
}
