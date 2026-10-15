/**
 * The shell's festivals, seasons and 节气 pictures — pure, so it is tested
 * without a page. The calendar is lunar.mts (a port of the Lingjing skill's);
 * the pictures ship in ui/public (festivals/, jieqi/), painted in the skill's
 * 水墨淡彩 house style. What shows behind the sky overlay, faintly: on a
 * festival's day its picture (春节's for 初一–初七), else the 节气 in effect.
 */
import { festivalOn, LUNAR, seasonOf, termAt, type Season, type Ymd } from './lunar.mts';

export interface FestivalMeta {
  /** Its name in the chip's title. */
  name: string;
  /** What drifts over the window that day (the overlay's own particle kinds). */
  particles: string | null;
  /** The first days of its span its picture shows (default 1). */
  artDays?: number;
}

/** Every festival the calendar names. Declarative: a new one is a row here. */
export const FESTIVALS: Record<string, FestivalMeta> = {
  chuxi: { name: "New Year's Eve", particles: 'sparks' },
  chunjie: { name: 'Spring Festival', particles: 'sparks', artDays: 7 },
  yuanxiao: { name: 'Lantern Festival', particles: 'glow' },
  qingrenjie: { name: "Valentine's Day", particles: 'petals' },
  qingming: { name: 'Qingming', particles: null },
  duanwu: { name: 'Dragon Boat Festival', particles: null },
  qixi: { name: 'Qixi', particles: 'stars' },
  zhongqiu: { name: 'Mid-Autumn', particles: 'glow' },
  chongyang: { name: 'Double Ninth', particles: 'leaves' },
  wanshengjie: { name: 'Halloween', particles: 'leaves' },
  ganenjie: { name: 'Thanksgiving', particles: 'leaves' },
  laba: { name: 'Laba', particles: null },
  yuandan: { name: "New Year's Day", particles: 'sparks' },
  shengdan: { name: 'Christmas', particles: 'snow' },
};

/** The pictures this build ships (ui/public/festivals/<id>.webp, ui/public/jieqi/<id>.webp). */
export const FESTIVAL_ART = new Set(['chuxi', 'chunjie', 'yuanxiao', 'qingrenjie', 'qingming', 'duanwu', 'qixi', 'zhongqiu', 'chongyang', 'wanshengjie', 'ganenjie', 'laba', 'yuandan', 'shengdan']);
export const TERM_ART = new Set<string>(['lichun', 'yushui', 'jingzhe', 'chunfen', 'qingming', 'guyu', 'lixia', 'xiaoman', 'mangzhong', 'xiazhi', 'xiaoshu', 'dashu', 'liqiu', 'chushu', 'bailu', 'qiufen', 'hanlu', 'shuangjiang', 'lidong', 'xiaoxue', 'daxue', 'dongzhi', 'xiaohan', 'dahan']);

/** A Date read in its own time zone as {y, m, d}. */
export const ymdOf = (now: Date): Ymd => ({ y: now.getFullYear(), m: now.getMonth() + 1, d: now.getDate() });

/** `2026-10-31` → {y, m, d}, or null. */
export function parseYmd(raw: string | null | undefined): Ymd | null {
  const m = /^(\d{4})-(\d{1,2})-(\d{1,2})$/.exec((raw ?? '').trim());
  return m ? { y: Number(m[1]), m: Number(m[2]), d: Number(m[3]) } : null;
}

export interface Today {
  /** Today's festival, with how far into its span (1 = its first day). */
  festival: { id: string; name: string; particles: string | null; day: number } | null;
  /** The 节气 in effect, from its day to the next. */
  term: { id: string; name: { zh: string; en: string } } | null;
  season: Season;
}

/** What the calendar says of a day where the person is (latitude, ISO country). */
export function todayOf(day: Ymd, where: { latitude?: number | null; country?: string | null } = {}): Today {
  const f = festivalOn(day.y, day.m, day.d, LUNAR, { country: where.country ?? null });
  const meta = f ? FESTIVALS[f.id] : null;
  return {
    festival: f && meta ? { id: f.id, name: meta.name, particles: meta.particles, day: f.day ?? 1 } : null,
    term: termAt(day.y, day.m, day.d),
    season: seasonOf(day, where.latitude ?? null),
  };
}

/** The faint picture behind the overlay: one ordered resolver — the festival's
 *  (within its art days), else the 节气's — or null when this build has neither. */
export const BACKDROP_TIERS: [string, (t: Today) => string | null][] = [
  ['festival', (t) => (t.festival && FESTIVAL_ART.has(t.festival.id) && t.festival.day <= (FESTIVALS[t.festival.id]?.artDays ?? 1) ? `festivals/${t.festival.id}.webp` : null)],
  ['term', (t) => (t.term && TERM_ART.has(t.term.id) ? `jieqi/${t.term.id}.webp` : null)],
];

export function backdropOf(t: Today): { tier: string; src: string } | null {
  for (const [tier, pick] of BACKDROP_TIERS) {
    const src = pick(t);
    if (src) return { tier, src };
  }
  return null;
}

/* Wind levels by strength, km/h — the sustained wind or 0.7 of its gusts,
   whichever is more (30 km/h gusting 62 reads windy). The skill's table
   (rules/weather.mjs WINDS), kept in step. */
export const WINDS = [
  { level: 'calm', upTo: 8, force: 0.08 },
  { level: 'breeze', upTo: 22, force: 0.3 },
  { level: 'windy', upTo: 45, force: 0.7 },
  { level: 'gale', upTo: Infinity, force: 1 },
] as const;

export interface Wind { level: string; force: number; dir?: number; kmh?: number; gust_kmh?: number }

/** A reading's wind, or null when it carries none. */
export function windOf(r: { wind_kmh?: number | null; gust_kmh?: number | null; wind_dir?: number | null } | null | undefined): Wind | null {
  const kmh = r?.wind_kmh ?? null, gust = r?.gust_kmh ?? null;
  if (kmh == null && gust == null) return null;
  const strength = Math.max(kmh ?? 0, (gust ?? 0) * 0.7);
  const row = WINDS.find((w) => strength < w.upTo) ?? WINDS[WINDS.length - 1];
  return {
    level: row.level, force: row.force,
    ...(r?.wind_dir != null ? { dir: r.wind_dir } : {}),
    ...(kmh != null ? { kmh: Math.round(kmh) } : {}),
    ...(gust != null ? { gust_kmh: Math.round(gust) } : {}),
  };
}

/** A wind level by name (the `?wind=` override). */
export function windNamed(level: string | null | undefined, dir?: number): Wind | null {
  const row = WINDS.find((w) => w.level === level);
  return row ? { level: row.level, force: row.force, ...(dir != null && Number.isFinite(dir) ? { dir } : {}) } : null;
}

/** The chip's wind words: "windy, 30 km/h, gusts 63" — or null when calm enough to leave out. */
export function windWords(w: Wind | null): string | null {
  if (!w || (w.level !== 'windy' && w.level !== 'gale')) return null;
  const speed = w.kmh != null ? `, ${w.kmh} km/h` : '';
  const gusts = w.gust_kmh != null ? `, gusts ${w.gust_kmh}` : '';
  return `${w.level === 'gale' ? 'Gale' : 'Windy'}${speed}${gusts}`;
}
