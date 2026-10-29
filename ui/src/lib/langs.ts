// BCP-47 codes from the contract, with display names per UI locale.

import { i18n } from './i18n.svelte';

export const LANGS = ['zh-Hans', 'zh-Hant', 'en', 'ja', 'ko', 'fr', 'de', 'es', 'ru', 'pt', 'it', 'vi', 'th', 'ar'] as const;
export const SOURCE_LANGS = ['auto', ...LANGS] as const;

const names: Record<string, [zh: string, en: string]> = {
  auto: ['自动检测', 'Detect'],
  'zh-Hans': ['简体中文', 'Chinese (Simplified)'],
  'zh-Hant': ['繁体中文', 'Chinese (Traditional)'],
  en: ['英语', 'English'],
  ja: ['日语', 'Japanese'],
  ko: ['韩语', 'Korean'],
  fr: ['法语', 'French'],
  de: ['德语', 'German'],
  es: ['西班牙语', 'Spanish'],
  ru: ['俄语', 'Russian'],
  pt: ['葡萄牙语', 'Portuguese'],
  it: ['意大利语', 'Italian'],
  vi: ['越南语', 'Vietnamese'],
  th: ['泰语', 'Thai'],
  ar: ['阿拉伯语', 'Arabic'],
};

/** Short names for the compact popup menu in English (keeps three columns). */
const shortEn: Record<string, string> = {
  'zh-Hans': 'Chinese (S)',
  'zh-Hant': 'Chinese (T)',
};

export function langName(code: string | null | undefined, short = false): string {
  if (!code) return '';
  const n = names[code] ?? names[code.split('-')[0]];
  if (!n) return code;
  if (i18n.locale === 'en-US') return (short && shortEn[code]) || n[1];
  return n[0];
}

/** `lang` attribute for content in a given language, so CJK text picks the right glyphs and font. */
export function htmlLang(code: string | null | undefined): string | undefined {
  if (!code || code === 'auto') return undefined;
  if (code === 'zh-Hans') return 'zh-CN';
  if (code === 'zh-Hant') return 'zh-TW';
  return code;
}
