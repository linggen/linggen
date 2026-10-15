/**
 * The festival calendar and the seasons, reckoned on the device — a port of
 * the Lingjing skill's scripts/calendar.js (the engine never imports a skill
 * at runtime). Lunar dates from a table (2025–2051, Beijing time); the 节气
 * by the sun's apparent longitude (Meeus ch. 25). Keep the two in step:
 * ui/tests/lunar.test.mts checks the same published dates.
 */

export interface Ymd { y: number; m: number; d: number }
export interface Term { id: string; name: { zh: string; en: string } }
export interface Lunar { year: number; month: number; day: number; leap: boolean }
export interface Festival { id: string; key: string; gift: string; day?: number }

/* Day numbers: the Julian Day Number of a civil date (noon), and back. */
export function jdOfDay(y: number, m: number, d: number): number {
  const a = Math.floor((14 - m) / 12), yy = y + 4800 - a, mm = m + 12 * a - 3;
  return d + Math.floor((153 * mm + 2) / 5) + 365 * yy + Math.floor(yy / 4) - Math.floor(yy / 100) + Math.floor(yy / 400) - 32045;
}
export function dayOfJd(n: number): Ymd {
  const a = n + 32044, b = Math.floor((4 * a + 3) / 146097), c = a - Math.floor(146097 * b / 4);
  const d0 = Math.floor((4 * c + 3) / 1461), e = c - Math.floor(1461 * d0 / 4), m0 = Math.floor((5 * e + 2) / 153);
  return { y: 100 * b + d0 - 4800 + Math.floor(m0 / 10), m: m0 + 3 - 12 * Math.floor(m0 / 10), d: e - Math.floor((153 * m0 + 2) / 5) + 1 };
}

const rad = Math.PI / 180;
/// The sun's apparent longitude in degrees at a Julian Day (TT).
export function sunLongitude(jde: number): number {
  const T = (jde - 2451545) / 36525;
  const L0 = 280.46646 + 36000.76983 * T + 0.0003032 * T * T;
  const M = (357.52911 + 35999.05029 * T - 0.0001537 * T * T) * rad;
  const C = (1.914602 - 0.004817 * T - 0.000014 * T * T) * Math.sin(M) + (0.019993 - 0.000101 * T) * Math.sin(2 * M) + 0.000289 * Math.sin(3 * M);
  const om = (125.04 - 1934.136 * T) * rad;
  return (((L0 + C - 0.00569 - 0.00478 * Math.sin(om)) % 360) + 360) % 360;
}

/// The twenty-four terms from 春分 (0°), 15° apart.
export const TERMS: Term[] = ([
  ['chunfen', '春分', 'Spring Equinox'], ['qingming', '清明', 'Clear and Bright'], ['guyu', '谷雨', 'Grain Rain'],
  ['lixia', '立夏', 'Start of Summer'], ['xiaoman', '小满', 'Grain Buds'], ['mangzhong', '芒种', 'Grain in Ear'],
  ['xiazhi', '夏至', 'Summer Solstice'], ['xiaoshu', '小暑', 'Minor Heat'], ['dashu', '大暑', 'Major Heat'],
  ['liqiu', '立秋', 'Start of Autumn'], ['chushu', '处暑', 'End of Heat'], ['bailu', '白露', 'White Dew'],
  ['qiufen', '秋分', 'Autumn Equinox'], ['hanlu', '寒露', 'Cold Dew'], ['shuangjiang', '霜降', 'Frost\'s Descent'],
  ['lidong', '立冬', 'Start of Winter'], ['xiaoxue', '小雪', 'Minor Snow'], ['daxue', '大雪', 'Major Snow'],
  ['dongzhi', '冬至', 'Winter Solstice'], ['xiaohan', '小寒', 'Minor Cold'], ['dahan', '大寒', 'Major Cold'],
  ['lichun', '立春', 'Start of Spring'], ['yushui', '雨水', 'Rain Water'], ['jingzhe', '惊蛰', 'Awakening of Insects'],
] as [string, string, string][]).map(([id, zh, en]) => ({ id, name: { zh, en } }));

const deltaT = (y: number): number => (69 + 0.35 * (y - 2026)) / 86400;

/// The 节气 that falls on this date (Beijing time), or null.
export function termOf(y: number, m: number, d: number): Term | null {
  const start = jdOfDay(y, m, d) - 0.5 - 8 / 24 + deltaT(y);
  const a = Math.floor(sunLongitude(start) / 15), b = Math.floor(sunLongitude(start + 1) / 15);
  return a === b ? null : TERMS[b];
}

/* Per lunar year: 正月初一 as MMDD, the leap month in hex (0 none), then each
   month in order, 1 = 30 days, 0 = 29 (tools/lunar-table.mjs). 2027 and 2030's
   正月初一 fall minutes either side of Beijing midnight; ICU's own Chinese
   calendar gets both a day wrong, this table has them as published. */
export const LUNAR: Record<number, string> = {
  2025: '012961010010101110', 2026: '02170101001001110', 2027: '02060110100100110', 2028: '012651110100100110',
  2029: '02130110101010011', 2030: '02030010110101010', 2031: '012330110101101010', 2032: '02110100101101101',
  2033: '0131b0100101011101', 2034: '02190010010101101', 2035: '02080101001001101', 2036: '012861101001001011',
  2037: '02150110100100101', 2038: '02040110101010010', 2039: '012451101101010100', 2040: '02120101101011010',
  2041: '02010010101101101', 2042: '012220100101011011', 2043: '02100010010011011', 2044: '013071010010010111',
  2045: '02170101001001011', 2046: '02060101010100101', 2047: '012651011010100101', 2048: '02140011011010010',
  2049: '02020101011011010', 2050: '012330101010110110', 2051: '02110100100110111',
};

/// The 节气 in effect on a date (Beijing time): the last one that fell on or
/// before it — a 节气 stands until the next (they come every 14–16 days).
export function termAt(y: number, m: number, d: number): Term | null {
  const n = jdOfDay(y, m, d);
  for (let back = 0; back < 17; back++) {
    const { y: yy, m: mm, d: dd } = dayOfJd(n - back);
    const t = termOf(yy, mm, dd);
    if (t) return t;
  }
  return null;
}

/// The lunar date of a civil date — {year, month, day, leap} — or null
/// outside the table.
export function lunarOf(y: number, m: number, d: number, rows: Record<number, string> = LUNAR): Lunar | null {
  const n = jdOfDay(y, m, d);
  for (const year of [y, y - 1]) {
    const row = rows[year];
    if (!row) continue;
    let at = jdOfDay(year, Number(row.slice(0, 2)), Number(row.slice(2, 4)));
    if (n < at) continue;
    const leap = parseInt(row[4], 16), lens = row.slice(5);
    let month = 0;
    for (let i = 0; i < lens.length; i++) {
      const isLeap = leap > 0 && i === leap; // the leap month follows month `leap`
      if (!isLeap) month++;
      const len = lens[i] === '1' ? 30 : 29;
      if (n < at + len) return { year, month, day: n - at + 1, leap: isLeap };
      at += len;
    }
  }
  return null;
}

/// A lunar date's civil date {y, m, d} (the regular month, never the leap).
export function solarOf(year: number, month: number, day: number, rows: Record<number, string> = LUNAR): Ymd | null {
  const row = rows[year];
  if (!row) return null;
  const leap = parseInt(row[4], 16), lens = row.slice(5);
  let at = jdOfDay(year, Number(row.slice(0, 2)), Number(row.slice(2, 4))), n = 0;
  for (let i = 0; i < lens.length; i++) {
    const isLeap = leap > 0 && i === leap;
    if (!isLeap) n++;
    if (n === month && !isLeap) return dayOfJd(at + day - 1);
    at += lens[i] === '1' ? 30 : 29;
  }
  return null;
}

/* The festivals by date: lunar (month/day), civil, by 节气, and by the nth
   weekday of a month where the day differs by country (感恩节: Canada's
   second Monday of October, the US's fourth Thursday of November; elsewhere
   none). 除夕 is the last day of the year; 春节's span runs 除夕 → 元宵, one
   festival (design: the custom). */
const LUNAR_DAYS: Record<string, string> = { '1/1': 'chunjie', '1/15': 'yuanxiao', '5/5': 'duanwu', '7/7': 'qixi', '8/15': 'zhongqiu', '9/9': 'chongyang', '12/8': 'laba' };
const CIVIL_DAYS: Record<string, string> = { '1/1': 'yuandan', '2/14': 'qingrenjie', '10/31': 'wanshengjie', '12/24': 'shengdan', '12/25': 'shengdan' };
const TERM_DAYS: Record<string, string> = { qingming: 'qingming' };
/* id → { country: [month, weekday (0 = Sunday), nth] }. */
const NTH_WEEKDAYS: Record<string, Record<string, [number, number, number]>> = { ganenjie: { CA: [10, 1, 2], US: [11, 4, 4] } };

/// Every festival id the calendar can name.
export const FESTIVAL_IDS = ['chuxi', 'chunjie', 'yuanxiao', ...new Set([...Object.values(LUNAR_DAYS), ...Object.values(CIVIL_DAYS), ...Object.values(TERM_DAYS), ...Object.keys(NTH_WEEKDAYS)])].filter((id, i, all) => all.indexOf(id) === i);

/// The day of the month of a month's nth weekday (0 = Sunday).
export function nthWeekday(y: number, m: number, weekday: number, nth: number): number {
  const first = (jdOfDay(y, m, 1) + 1) % 7; // JDN + 1 mod 7: 0 = Sunday
  return 1 + ((weekday - first + 7) % 7) + 7 * (nth - 1);
}

/* A festival kept by the weekday where the player is (`country`, ISO code). */
function weekdayFestival(y: number, m: number, d: number, country: string | null | undefined): string | null {
  for (const [id, by] of Object.entries(NTH_WEEKDAYS)) {
    const rule = by[String(country ?? '').toUpperCase()];
    if (rule && rule[0] === m && nthWeekday(y, m, rule[1], rule[2]) === d) return id;
  }
  return null;
}

/// Today's festival — {id, key, gift, day?} — or null. `key` names this
/// year's one (its task is done once); `gift` the span its gift belongs to
/// (除夕 → 元宵 is one 春节); `day` is where in that span it stands.
/// `where.country` (an ISO code, the weather sense's) decides a festival
/// whose day is the country's.
export function festivalOn(y: number, m: number, d: number, rows: Record<number, string> = LUNAR, where: { country?: string | null } = {}): Festival | null {
  // The lunar feasts first: a western wink never takes a day of the 春节 span.
  const lunar = lunarFestival(y, m, d, rows);
  if (lunar) return lunar;
  const id = CIVIL_DAYS[`${m}/${d}`] ?? weekdayFestival(y, m, d, where.country) ?? TERM_DAYS[termOf(y, m, d)?.id ?? ''];
  return id ? { id, key: `${id}-${y}`, gift: `${id}-${y}` } : null;
}

/* The lunar calendar's festival of a civil date, or null. */
function lunarFestival(y: number, m: number, d: number, rows: Record<number, string>): Festival | null {
  const l = lunarOf(y, m, d, rows);
  if (!l) return null;
  const n = jdOfDay(y, m, d), next = rows[l.year + 1];
  const spring = (id: string, year: number, day: number | null = null): Festival => ({ id, key: `${id}-${year}`, gift: `chunjie-${year}`, ...(day ? { day } : {}) });
  if (next && n === jdOfDay(l.year + 1, Number(next.slice(0, 2)), Number(next.slice(2, 4))) - 1) return spring('chuxi', l.year + 1);
  if (l.leap) return null;
  const id = LUNAR_DAYS[`${l.month}/${l.day}`];
  if (id === 'chunjie' || id === 'yuanxiao') return spring(id, l.year);
  if (id) return { id, key: `${id}-${l.year}`, gift: `${id}-${l.year}` };
  if (l.month === 1 && l.day < 15) return spring('chunjie', l.year, l.day);
  return null;
}

/* The seasons by the sun, as the 节气 reckon them: 春 from 立春 (315°), 夏
   from 立夏 (45°), 秋 from 立秋 (135°), 冬 from 立冬 (225°) — in the south,
   half a year round. */
export const SEASONS = ['spring', 'summer', 'autumn', 'winter'] as const;
export type Season = (typeof SEASONS)[number];

/// The season of a civil date where the latitude is (north when unknown).
export function seasonOf({ y, m, d }: Ymd, latitude: number | null = null): Season {
  const lon = sunLongitude(jdOfDay(y, m, d) + deltaT(y));
  const south = latitude != null && Number.isFinite(latitude) && latitude < 0;
  const q = Math.floor((((lon + (south ? 180 : 0) + 45) % 360) / 90)); // 0 = 立春's quarter
  return SEASONS[q];
}
