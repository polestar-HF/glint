// Hand-drawn line icons on a 16 × 16 grid: 1.5 px stroke, round caps and joins.
// Each icon is a list of parts; `f` parts are filled (dots, solid halves) instead of stroked.
// Keep glyphs inside 1.75–14.25 so the stroke never touches the box edge.

export interface IconPart {
  d: string;
  f?: true;
}
export interface IconDef {
  parts: IconPart[];
  /** Optional rotation in degrees around the center. */
  rotate?: number;
}

const s = (...ds: string[]): IconPart[] => ds.map((d) => ({ d }));
const dot = (cx: number, cy: number, r = 0.875): IconPart => ({
  d: `M${cx} ${cy - r}a${r} ${r} 0 1 1 0 ${2 * r}a${r} ${r} 0 1 1 0-${2 * r}Z`,
  f: true,
});
const circle = 'M8 1.75a6.25 6.25 0 1 1 0 12.5a6.25 6.25 0 1 1 0-12.5Z';
const pinBody = 'M6.5 2.75V6.5L4.25 9.25h7.5L9.5 6.5V2.75';

export const icons = {
  'chevron-down': { parts: s('M4.5 6.25 8 9.75l3.5-3.5') },
  'chevron-up': { parts: s('M4.5 9.75 8 6.25l3.5 3.5') },
  'chevron-right': { parts: s('M6.25 4.5 9.75 8l-3.5 3.5') },
  'chevron-small': { parts: s('M5.25 6.75 8 9.5l2.75-2.75') },
  swap: { parts: s('M2.75 5.25h10.5', 'M10.5 2.5l2.75 2.75L10.5 8', 'M13.25 10.75H2.75', 'M5.5 8l-2.75 2.75L5.5 13.5') },
  copy: {
    parts: s(
      'M6.75 5.25h5a1.5 1.5 0 0 1 1.5 1.5v5a1.5 1.5 0 0 1-1.5 1.5h-5a1.5 1.5 0 0 1-1.5-1.5v-5a1.5 1.5 0 0 1 1.5-1.5Z',
      'M10.75 5.25v-1a1.5 1.5 0 0 0-1.5-1.5h-5a1.5 1.5 0 0 0-1.5 1.5v5a1.5 1.5 0 0 0 1.5 1.5h1',
    ),
  },
  close: { parts: s('M4.25 4.25l7.5 7.5', 'M11.75 4.25l-7.5 7.5') },
  pin: { parts: s('M5.75 2.75h4.5', pinBody, 'M8 9.25v4'), rotate: 45 },
  'pin-filled': { parts: [{ d: pinBody + 'Z', f: true }, ...s('M5.75 2.75h4.5', pinBody + 'Z', 'M8 9.25v4')] },
  check: { parts: s('M3.5 8.25 6.5 11.25 12.5 4.75') },
  alert: { parts: [...s(circle, 'M8 4.75v3.75'), dot(8, 11)] },
  info: { parts: [...s(circle, 'M8 7.25V11'), dot(8, 5)] },
  retry: { parts: s('M13.5 8a5.5 5.5 0 1 1-5.5-5.5c1.54 0 3.02.61 4.12 1.68L13.5 5', 'M13.5 2.25V5h-2.75') },
  reset: { parts: s('M2.5 8A5.5 5.5 0 1 0 8 2.5c-1.54 0-3.02.61-4.12 1.68L2.5 5', 'M2.5 2.25V5h2.75') },
  enter: { parts: s('M13 3.5V7a2 2 0 0 1-2 2H3.5', 'M6.25 6.25 3.5 9l2.75 2.75') },
  keyboard: {
    parts: s(
      'M3.25 3.75h9.5a1.5 1.5 0 0 1 1.5 1.5v5.5a1.5 1.5 0 0 1-1.5 1.5h-9.5a1.5 1.5 0 0 1-1.5-1.5v-5.5a1.5 1.5 0 0 1 1.5-1.5Z',
      'M4.75 6.5h.01M7.25 6.5h.01M9.75 6.5h.01M11.25 6.5h.01',
      'M5.75 9.25h4.5',
    ),
  },
  sliders: {
    parts: s(
      'M2.75 5h4.75',
      'M10 3.5a1.5 1.5 0 1 1 0 3a1.5 1.5 0 1 1 0-3Z',
      'M12.5 5h.75',
      'M2.75 11h.75',
      'M6 9.5a1.5 1.5 0 1 1 0 3a1.5 1.5 0 1 1 0-3Z',
      'M8.5 11h4.75',
    ),
  },
  scan: {
    parts: s(
      'M2.75 5.5V4.25a1.5 1.5 0 0 1 1.5-1.5H5.5',
      'M10.5 2.75h1.25a1.5 1.5 0 0 1 1.5 1.5V5.5',
      'M13.25 10.5v1.25a1.5 1.5 0 0 1-1.5 1.5H10.5',
      'M5.5 13.25H4.25a1.5 1.5 0 0 1-1.5-1.5V10.5',
      'M5.75 6.5h4.5',
      'M5.75 9.5h3',
    ),
  },
  translate: {
    parts: s(
      'M5.25 1.75V3',
      'M2 3.5h6.5',
      'M3.6 3.5c.7 2.45 2.2 4.15 4.4 5.1',
      'M6.9 3.5c-.6 2.35-2.05 4.1-4.65 5.1',
      'M8.75 14.25 11.25 7.5l2.5 6.75',
      'M9.6 12h3.3',
    ),
  },
  contrast: { parts: [...s(circle), { d: 'M8 1.75v12.5a6.25 6.25 0 1 0 0-12.5Z', f: true }] },
  grip: { parts: [dot(6, 4, 1), dot(10, 4, 1), dot(6, 8, 1), dot(10, 8, 1), dot(6, 12, 1), dot(10, 12, 1)] },
  external: {
    parts: s(
      'M9.25 2.75h4v4',
      'M13.25 2.75 7.75 8.25',
      'M11.75 9.5v2.25a1.5 1.5 0 0 1-1.5 1.5h-6a1.5 1.5 0 0 1-1.5-1.5v-6a1.5 1.5 0 0 1 1.5-1.5H6.5',
    ),
  },
  eye: {
    parts: s('M1.75 8S4 3.75 8 3.75 14.25 8 14.25 8 12 12.25 8 12.25 1.75 8 1.75 8Z', 'M8 6a2 2 0 1 1 0 4a2 2 0 1 1 0-4Z'),
  },
  'eye-off': {
    parts: s(
      'M6.6 3.9A6.6 6.6 0 0 1 8 3.75c4 0 6.25 4.25 6.25 4.25a11 11 0 0 1-1.6 2.2',
      'M10.1 10.65A5.4 5.4 0 0 1 8 12.25C4 12.25 1.75 8 1.75 8a11.3 11.3 0 0 1 2.4-2.9',
      'M2.75 2.75l10.5 10.5',
    ),
  },
  'arrow-up': { parts: s('M8 13V3.5', 'M4.25 7.25 8 3.5l3.75 3.75') },
  'arrow-down': { parts: s('M8 3v9.5', 'M4.25 8.75 8 12.5l3.75-3.75') },
} satisfies Record<string, IconDef>;

export type IconName = keyof typeof icons;
