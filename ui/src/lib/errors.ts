// Localized text for translation errors. The backend sends a stable `code`
// (glint_translate::TranslateError::code) next to its short English message; the message
// stays available as a tooltip for details.

import { t, type MessageKey } from './i18n.svelte';

export interface ErrorInfo {
  code?: string | null;
  message: string;
  retryAfterSecs?: number | null;
}

const KNOWN = new Set([
  'rate_limited', 'quota', 'auth', 'not_found', 'too_long', 'bad_request',
  'server', 'timeout', 'network', 'config', 'bad_response',
]);

function eta(secs: number): string {
  return secs < 60 ? t('err.secs', { n: Math.max(1, Math.round(secs)) }) : t('err.mins', { n: Math.ceil(secs / 60) });
}

/**
 * `freeEndpoint`: the engine runs on an unofficial free endpoint (Google/Bing without a key),
 * which gets a more specific rate-limit hint.
 */
export function describeError(e: ErrorInfo, freeEndpoint = false): { text: string; hint: string | null } {
  const code = e.code ?? '';
  if (!KNOWN.has(code)) return { text: e.message, hint: null };
  if (code === 'rate_limited') {
    const text = e.retryAfterSecs ? t('err.rate_limited_eta', { eta: eta(e.retryAfterSecs) }) : t('err.rate_limited');
    return { text, hint: t(freeEndpoint ? 'err.hint.rate_free' : 'err.hint.rate') };
  }
  const hints: Record<string, MessageKey> = {
    quota: 'err.hint.quota',
    auth: 'err.hint.auth',
    network: 'err.hint.network',
    config: 'err.hint.config',
  };
  return { text: t(`err.${code}` as MessageKey), hint: hints[code] ? t(hints[code]) : null };
}

/** Errors from `test_engine` arrive as `{ code, message, retryAfterSecs }` (or a plain string). */
export function asErrorInfo(err: unknown): ErrorInfo {
  if (err && typeof err === 'object' && 'message' in err) {
    const o = err as { code?: string; message?: string; retryAfterSecs?: number };
    return { code: o.code ?? null, message: String(o.message ?? ''), retryAfterSecs: o.retryAfterSecs ?? null };
  }
  return { code: null, message: String(err) };
}
