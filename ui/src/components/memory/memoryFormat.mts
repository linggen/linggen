/** How the Memory tab words a row's facts. Pure, so node can test it. */

import type { Tier } from './memoryApi.mts';

const MINUTE = 60_000;
const UNITS: [string, number][] = [['mo', 30 * 86_400_000], ['w', 7 * 86_400_000], ['d', 86_400_000], ['h', 3_600_000], ['m', MINUTE]];

/** "3d ago" from an ISO time; empty when there is none. */
export function relAge(iso: string | null | undefined, now = Date.now()): string {
  const t = iso ? Date.parse(iso) : NaN;
  if (Number.isNaN(t)) return '';
  const delta = Math.max(0, now - t);
  const unit = UNITS.find(([, ms]) => delta >= ms);
  return unit ? `${Math.floor(delta / unit[1])}${unit[0]} ago` : 'just now';
}

export function fmtBytes(b: number): string {
  if (b >= 1e9) return `${(b / 1e9).toFixed(1)} GB`;
  if (b >= 1e6) return `${(b / 1e6).toFixed(1)} MB`;
  return `${Math.round(b / 1e3)} KB`;
}

/** A directory as people read it: `~/…` under a home folder. */
export function shortPath(p: string | null | undefined): string {
  return String(p ?? '').replace(/^\/(Users|home)\/[^/]+/, '~');
}

/** The tier as the tabs name it. */
export function tierLabel(t: Tier | string | undefined): string {
  return t === 'semantic' ? 'Long-term' : t === 'core' ? 'Core' : t === 'episodic' ? 'Episodic' : '';
}

/** The first line or so of a row, for the list. */
export function preview(content: string, max = 220): string {
  const flat = content.replace(/\s+/g, ' ').trim();
  return flat.length > max ? `${flat.slice(0, max - 1)}…` : flat;
}

/** The language the page is in (`lib/i18n` sets it on <html>), as a date locale;
 *  undefined = the browser's own. */
export function uiLocale(): string | undefined {
  return typeof document !== 'undefined' && document.documentElement.lang === 'zh-Hans' ? 'zh-CN' : undefined;
}

/** "1 row", "3 rows". */
export function rowCount(n: number): string {
  return `${n} row${n === 1 ? '' : 's'}`;
}

/** A local calendar day as a short readable date, or the original text. */
export function fmtTime(iso: string | null | undefined): string {
  const t = iso ? new Date(iso) : null;
  return t && !Number.isNaN(t.getTime()) ? t.toLocaleString(uiLocale()) : '';
}
