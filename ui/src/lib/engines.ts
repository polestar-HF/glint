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
    required: ['apiKey'],
    fields: [
      { key: 'apiKey', label: 'field.apiKey', secret: true, placeholder: 'sk-ant-…', hint: 'hint.anthropicKey' },
      { key: 'model', label: 'field.model', placeholder: 'claude-haiku-5-5' },
      { key: 'baseUrl', label: 'field.baseUrl', optional: true, placeholder: 'https://api.anthropic.com' },
      prompt,
    ],
  },
  openai: {
    free: false,
    required: ['baseUrl'],
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

export function isConfigured(e: EngineConfig): boolean {
  return kindSpec(e.kind).required.every((k) => !!(e[k] && String(e[k]).trim()));
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
    ui: { theme: 'system', fontSize: 14, accent: '#1E6AC5', closeOnBlur: false, popupIdleDestroySecs: 600 },
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
