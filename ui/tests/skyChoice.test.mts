// node --test ui/tests/*.test.mts (Node strips the types).
// The shell's sky: which weather it draws, and the Settings line.
import { test } from 'node:test';
import assert from 'node:assert/strict';
import { parseForce, placeLine, skyOfView } from '../src/lib/skyChoice.mts';

test('the override names a kind and maybe night', () => {
  assert.deepEqual(parseForce('rain'), { kind: 'rain', is_day: true });
  assert.deepEqual(parseForce('Snow-night'), { kind: 'snow', is_day: false });
  assert.equal(parseForce('hail'), null);
  assert.equal(parseForce(null), null);
});

test('a reading becomes a sky; no reading, no sky', () => {
  assert.deepEqual(skyOfView({ reading: { kind: 'storm', is_day: false } }), { kind: 'storm', is_day: false });
  assert.equal(skyOfView({ reading: null }), null);
  assert.equal(skyOfView(null), null);
});

test('the place line says where and whether it was found or typed', () => {
  assert.equal(placeLine({ city: { name: 'Halifax', country: 'CA' }, auto: true }), 'Halifax, CA · automatic');
  assert.equal(placeLine({ city: { name: 'Harbin', country: '中国' }, auto: false }), 'Harbin, 中国');
  assert.equal(placeLine({ city: null, auto: true }), 'Finding your city… · automatic');
  assert.equal(placeLine({ off: true }), 'Weather is off');
});

test('the whole sky: the weather with its wind, the season where the city is, the festival\'s particles', async () => {
  const { composeSky, parseOverrides } = await import('../src/lib/skyChoice.mts');
  const { todayOf } = await import('../src/lib/skyArt.mts');
  const reading = { kind: 'clear', is_day: true, wind_kmh: 30.4, gust_kmh: 62.6, wind_dir: 317 };
  const view = { reading, city: { name: 'Halifax' }, off: false };
  const today = todayOf({ y: 2026, m: 10, d: 31 }, { latitude: 44.6 });
  const sky = composeSky(view, today, null);
  assert.equal(sky?.kind, 'clear');
  assert.equal(sky?.wind?.level, 'windy');
  assert.equal(sky?.season, 'autumn');
  assert.equal(sky?.festival, 'leaves'); // 万圣节
  // No reading, a festival: it still draws (kind none); no city, no season.
  const bare = composeSky(null, todayOf({ y: 2026, m: 9, d: 25 }), null);
  assert.deepEqual([bare?.kind, bare?.festival, bare?.season], ['none', 'glow', null]);
  assert.equal(composeSky(null, todayOf({ y: 2026, m: 9, d: 29 }), null), null, 'nothing to draw');
  // The dev overrides.
  const o = parseOverrides('?sky=rain&wind=gale&winddir=270&season=spring&festival=zhongqiu&date=2026-10-31');
  assert.deepEqual(o, { wind: { level: 'gale', force: 1, dir: 270 }, season: 'spring', festival: 'zhongqiu', date: '2026-10-31' });
  assert.deepEqual(parseOverrides('?wind=hurricane&season=monsoon'), {});
  const forced = composeSky(view, today, { kind: 'rain', is_day: false }, o);
  assert.deepEqual([forced?.kind, forced?.wind?.force, forced?.season, forced?.festival], ['rain', 1, 'spring', 'glow']);
});
