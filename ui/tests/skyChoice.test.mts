// node --test ui/tests/*.test.mts (Node strips the types).
// The shell's sky: when it shows, frosted or solid, and the Settings line.
import { test } from 'node:test';
import assert from 'node:assert/strict';
import { parseForce, placeLine, skyMode, skyOfView } from '../src/lib/skyChoice.mts';

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

test('off or unknown weather shows nothing; still or no frost shows solid panels', () => {
  const sky = { kind: 'rain', is_day: true };
  assert.equal(skyMode({ enabled: false, sky, frost: true, still: false }), null);
  assert.equal(skyMode({ enabled: true, sky: null, frost: true, still: false }), null);
  assert.equal(skyMode({ enabled: true, sky, frost: true, still: false }), 'glass');
  assert.equal(skyMode({ enabled: true, sky, frost: false, still: false }), 'solid');
  assert.equal(skyMode({ enabled: true, sky, frost: true, still: true }), 'solid');
});

test('the place line says where and whether it was found or typed', () => {
  assert.equal(placeLine({ city: { name: 'Halifax', country: 'CA' }, auto: true }), 'Halifax, CA · automatic');
  assert.equal(placeLine({ city: { name: 'Harbin', country: '中国' }, auto: false }), 'Harbin, 中国');
  assert.equal(placeLine({ city: null, auto: true }), 'Finding your city… · automatic');
  assert.equal(placeLine({ off: true }), 'Weather is off');
});
