// node --test ui/tests/*.test.mts (Node strips the types).
// The shell's calendar (a port of the Lingjing skill's scripts/calendar.js):
// published dates, the four western-and-term days, the 节气 in effect, the
// seasons by hemisphere — and what the shell shows for each.
import { test } from 'node:test';
import assert from 'node:assert/strict';
import { festivalOn, LUNAR, lunarOf, nthWeekday, seasonOf, termAt, termOf } from '../src/lib/lunar.mts';
import { backdropOf, BACKDROP_TIERS, FESTIVAL_ART, FESTIVALS, todayOf, windOf, windWords } from '../src/lib/skyArt.mts';

const ymd = (iso: string) => { const [y, m, d] = iso.split('-').map(Number); return { y, m, d }; };
const fest = (iso: string, country: string | null = null) => { const { y, m, d } = ymd(iso); return festivalOn(y, m, d, LUNAR, { country })?.id ?? null; };

test('the lunar feasts on their published days', () => {
  assert.equal(fest('2026-02-16'), 'chuxi');
  assert.equal(fest('2026-02-17'), 'chunjie');
  assert.equal(fest('2027-02-06'), 'chunjie');
  assert.equal(fest('2026-03-03'), 'yuanxiao');
  assert.equal(fest('2026-06-19'), 'duanwu');
  assert.equal(fest('2026-08-19'), 'qixi');
  assert.equal(fest('2026-09-25'), 'zhongqiu');
  assert.equal(fest('2026-10-18'), 'chongyang');
  assert.equal(fest('2027-01-15'), 'laba');
  assert.equal(fest('2026-09-29'), null);
  // A leap month's day is no festival: 2028 has a leap fifth month.
  assert.equal(lunarOf(2028, 6, 28)?.leap, true);
});

test('the western days and 清明; 感恩节 by the country; a wink never takes a 春节 day', () => {
  assert.equal(fest('2026-02-14'), 'qingrenjie');
  assert.equal(fest('2026-10-31'), 'wanshengjie');
  assert.equal(fest('2026-12-25'), 'shengdan');
  assert.equal(fest('2027-01-01'), 'yuandan');
  assert.equal(fest('2026-04-05'), 'qingming');
  assert.equal(fest('2026-10-12', 'CA'), 'ganenjie');
  assert.equal(fest('2026-11-26', 'US'), 'ganenjie');
  assert.equal(fest('2026-10-12', 'US'), null);
  assert.equal(fest('2026-10-12'), null);
  assert.equal(nthWeekday(2027, 11, 4, 4), 25);
  assert.equal(fest('2029-02-14'), 'chunjie');
  for (const id of ['chuxi', 'chunjie', 'yuanxiao', 'qingrenjie', 'qingming', 'duanwu', 'qixi', 'zhongqiu', 'chongyang', 'wanshengjie', 'ganenjie', 'laba', 'yuandan', 'shengdan']) assert.ok(FESTIVALS[id], id);
});

test('a 节气 stands from its day to the next; the seasons turn at 立春 立夏 立秋 立冬, half a year round in the south', () => {
  assert.equal(termOf(2026, 10, 8)?.id, 'hanlu');
  assert.equal(termAt(2026, 10, 7)?.id, 'qiufen');
  assert.equal(termAt(2026, 10, 15)?.id, 'hanlu');
  assert.equal(termAt(2027, 1, 1)?.id, 'dongzhi');
  assert.equal(seasonOf(ymd('2026-10-15'), 44.6), 'autumn');
  assert.equal(seasonOf(ymd('2026-10-15'), -33.9), 'spring');
  assert.equal(seasonOf(ymd('2026-02-05')), 'spring');
  assert.equal(seasonOf(ymd('2026-11-08')), 'winter');
});

test('the faint picture: the festival\'s on its day (春节 初一–初七), else the 节气\'s, else none', () => {
  const at = (iso: string) => backdropOf(todayOf(ymd(iso), { latitude: 44.6, country: 'CA' }));
  assert.deepEqual(at('2026-09-25'), { tier: 'festival', src: 'festivals/zhongqiu.webp' });
  assert.deepEqual(at('2027-02-06'), { tier: 'festival', src: 'festivals/chunjie.webp' });
  assert.deepEqual(at('2027-02-12'), { tier: 'festival', src: 'festivals/chunjie.webp' }, '初七');
  assert.notEqual(at('2027-02-13')?.tier, 'festival', '初八: the 节气 again');
  assert.deepEqual(at('2027-02-20'), { tier: 'festival', src: 'festivals/yuanxiao.webp' });
  // The tiers are data, in order.
  assert.deepEqual(BACKDROP_TIERS.map(([t]) => t), ['festival', 'term']);
  // A festival this build has no picture for falls through.
  assert.equal(FESTIVAL_ART.has('ganenjie') || at('2026-10-12')?.tier !== 'festival', true);
});

test('the wind: 30 km/h gusting 62 reads windy; the chip says so; calm says nothing', () => {
  const w = windOf({ wind_kmh: 30.4, gust_kmh: 62.6, wind_dir: 317 });
  assert.deepEqual(w, { level: 'windy', force: 0.7, dir: 317, kmh: 30, gust_kmh: 63 });
  assert.equal(windWords(w), 'Windy, 30 km/h, gusts 63');
  assert.equal(windWords(windOf({ wind_kmh: 6, gust_kmh: 9 })), null);
  assert.equal(windOf({}), null);
  assert.equal(windOf({ wind_kmh: 70, gust_kmh: 95 })?.level, 'gale');
});
