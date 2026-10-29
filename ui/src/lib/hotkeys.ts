// Accelerator strings, stored as e.g. "Alt+Q", "Ctrl+Shift+F1", "Super+Y".
// Order: Ctrl, Alt, Shift, Super; then one key named after KeyboardEvent.code
// (A–Z, 0–9, F1–F24, Space, PrintScreen, Up, Minus, Backquote, Numpad1, …).
// This is the format tauri-plugin-global-shortcut / global-hotkey parses.

import type { MessageKey } from './i18n.svelte';

export interface Chord {
  ctrl: boolean;
  alt: boolean;
  shift: boolean;
  win: boolean;
  key: string | null;
}

const MODS = ['Ctrl', 'Alt', 'Shift', 'Super'] as const;

export function parseChord(acc: string): Chord {
  const c: Chord = { ctrl: false, alt: false, shift: false, win: false, key: null };
  for (const raw of acc.split('+').map((s) => s.trim()).filter(Boolean)) {
    const p = raw.toLowerCase();
    if (p === 'ctrl' || p === 'control' || p === 'commandorcontrol') c.ctrl = true;
    else if (p === 'alt' || p === 'option') c.alt = true;
    else if (p === 'shift') c.shift = true;
    else if (p === 'super' || p === 'win' || p === 'meta' || p === 'cmd' || p === 'command') c.win = true;
    else c.key = normalizeKey(raw);
  }
  return c;
}

function normalizeKey(k: string): string {
  if (/^key[a-z]$/i.test(k)) return k.slice(3).toUpperCase();
  if (/^digit\d$/i.test(k)) return k.slice(5);
  if (/^arrow(up|down|left|right)$/i.test(k)) return cap(k.slice(5));
  return k.length === 1 ? k.toUpperCase() : k;
}
const cap = (s: string) => s.charAt(0).toUpperCase() + s.slice(1).toLowerCase();

export function formatChord(c: Chord): string {
  const parts: string[] = [];
  if (c.ctrl) parts.push(MODS[0]);
  if (c.alt) parts.push(MODS[1]);
  if (c.shift) parts.push(MODS[2]);
  if (c.win) parts.push(MODS[3]);
  if (c.key) parts.push(c.key);
  return parts.join('+');
}

const DISPLAY: Record<string, string> = {
  Super: 'Win',
  PrintScreen: 'PrtSc',
  Up: '↑',
  Down: '↓',
  Left: '←',
  Right: '→',
  PageUp: 'PgUp',
  PageDown: 'PgDn',
  Delete: 'Del',
  Insert: 'Ins',
  Backquote: '`',
  Minus: '-',
  Equal: '=',
  BracketLeft: '[',
  BracketRight: ']',
  Backslash: '\\',
  Semicolon: ';',
  Quote: "'",
  Comma: ',',
  Period: '.',
  Slash: '/',
};

/** Keycap labels for display, e.g. "Alt+Shift+W" → ["Alt", "Shift", "W"]. */
export function chordParts(acc: string): string[] {
  if (!acc) return [];
  const c = parseChord(acc);
  return formatChord(c)
    .split('+')
    .filter(Boolean)
    .map((p) => DISPLAY[p] ?? p.replace(/^Numpad/, 'Num '));
}

/** Maps a keydown to a key name, or null for modifiers and keys we don't accept. */
export function keyFromEvent(e: KeyboardEvent): string | null {
  const code = e.code;
  let m: RegExpMatchArray | null;
  if ((m = code.match(/^Key([A-Z])$/))) return m[1];
  if ((m = code.match(/^Digit(\d)$/))) return m[1];
  if (/^F([1-9]|1\d|2[0-4])$/.test(code)) return code;
  if ((m = code.match(/^Arrow(Up|Down|Left|Right)$/))) return m[1];
  if (/^Numpad(\d|Add|Subtract|Multiply|Divide|Decimal)$/.test(code)) return code;
  if (
    [
      'Space', 'PrintScreen', 'Home', 'End', 'PageUp', 'PageDown', 'Insert', 'Delete', 'Pause',
      'Backquote', 'Minus', 'Equal', 'BracketLeft', 'BracketRight', 'Backslash', 'Semicolon',
      'Quote', 'Comma', 'Period', 'Slash',
    ].includes(code)
  )
    return code;
  return null;
}

export function chordFromEvent(e: KeyboardEvent): Chord {
  return { ctrl: e.ctrlKey, alt: e.altKey, shift: e.shiftKey, win: e.metaKey, key: keyFromEvent(e) };
}

const RESERVED = new Set([
  'Alt+Tab', 'Alt+F4', 'Alt+Escape', 'Alt+Space', 'Ctrl+Escape', 'Ctrl+Shift+Escape', 'Ctrl+Alt+Delete',
  'Super+Shift+S', 'Super+Tab', 'Super+Space', 'Super+Period', 'Super+Comma',
  ...'LDERIXASVMPKHGBUCN1234567890'.split('').map((k) => `Super+${k}`),
  'Super+Up', 'Super+Down', 'Super+Left', 'Super+Right', 'Super+PrintScreen',
]);
const EDITING = new Set('CVXZYASFPWTNO'.split('').map((k) => `Ctrl+${k}`));

export interface Problem {
  key: MessageKey;
  vars?: Record<string, string>;
}

/**
 * Local validation. Registration conflicts with other apps are checked by the
 * backend (`check_hotkey`, contract change request #4).
 */
export function validateChord(c: Chord, others: Array<{ name: string; acc: string }>): Problem | null {
  if (!c.key) return null;
  const acc = formatChord(c);
  const hasMod = c.ctrl || c.alt || c.shift || c.win;
  const bareOk = /^(F1[3-9]|F2[0-4]|PrintScreen|Pause)$/.test(c.key);
  if (!hasMod && !bareOk) return { key: 'hk.needModifier' };
  if (c.shift && !c.ctrl && !c.alt && !c.win && !/^F\d+$/.test(c.key)) return { key: 'hk.shiftOnly' };
  if (RESERVED.has(acc)) return { key: 'hk.reserved' };
  if (EDITING.has(acc)) return { key: 'hk.editing' };
  const dup = others.find((o) => o.acc && formatChord(parseChord(o.acc)) === acc);
  if (dup) return { key: 'hk.duplicate', vars: { name: dup.name } };
  return null;
}
