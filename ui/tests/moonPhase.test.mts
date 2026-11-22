// node --test ui/tests/*.test.mts (Node strips the types).
// The moon in the page's sky: one function (shared/sky.js `moonPhase`) says how
// much of the disc is lit and whether it is waxing. Checked against its own
// definition, then against the lunar calendar the page already has.
import { test } from 'node:test';
import assert from 'node:assert/strict';
import { moonPhase } from '../../shared/sky.js';
import { dayOfJd, jdOfDay, lunarOf } from '../src/lib/lunar.mts';

const NEW_MOON = Date.UTC(2000, 0, 6, 18, 14);
const MONTH_MS = 29.530588853 * 86400000;

test('at its reference new moon: nothing lit, waxing; half a month later: all lit', () => {
  const now = moonPhase(NEW_MOON);
  assert.ok(now.lit < 1e-9);
  assert.equal(now.waxing, true);
  assert.ok(moonPhase(NEW_MOON + MONTH_MS / 2).lit > 1 - 1e-9);
  assert.ok(moonPhase(NEW_MOON + MONTH_MS * 300).lit < 1e-6, 'a whole number of months later it is new again');
  assert.ok(moonPhase(NEW_MOON - MONTH_MS * 7).lit < 1e-6, 'and before the reference too');
});

test('it rises and then falls once a month; waxing while it rises', () => {
  let turns = 0, prev = moonPhase(NEW_MOON).lit, prevWaxing = true, rising = true;
  const step = 3600000;
  for (let t = NEW_MOON + step; t <= NEW_MOON + MONTH_MS * 12; t += step) {
    const { lit, waxing } = moonPhase(t);
    if (Math.abs(lit - prev) > 1e-9) {
      const up = lit > prev;
      // (a step that straddles the peak or the trough has no one direction)
      if (waxing === prevWaxing) assert.equal(waxing, up, `at ${new Date(t).toISOString()} waxing says ${waxing} while lit goes ${up ? 'up' : 'down'}`);
      if (up !== rising) { turns += 1; rising = up; }
    }
    prev = lit; prevWaxing = waxing;
  }
  assert.equal(turns, 12 * 2 - 1, 'twelve months: twelve peaks and eleven troughs between them');
});

test('a Date is the same as its milliseconds; none given is now', () => {
  const t = Date.UTC(2026, 10, 21, 12);
  assert.deepEqual(moonPhase(new Date(t)), moonPhase(t));
  const before = Date.now(), p = moonPhase(), after = Date.now();
  assert.ok(p.lit >= 0 && p.lit <= 1 && [moonPhase(before).lit, moonPhase(after).lit].every((l) => Math.abs(l - p.lit) < 1e-3));
});

/* The page's own calendar (lunar.mts, Beijing time) against the phase at 20:00
   Beijing time, every day of two years. A lunar day is about 12° of phase wide
   and the new moon falls anywhere in 初一, so the bands are the physically true
   ones: the 35–65% a first quarter suggests is not true of 初七 or 廿四. */
test('the lunar calendar and the phase agree, every day of two years', () => {
  const bad: string[] = [];
  for (let n = jdOfDay(2026, 1, 1); n < jdOfDay(2028, 1, 1); n++) {
    const { y, m, d } = dayOfJd(n);
    const day = lunarOf(y, m, d)?.day;
    if (!day) continue;
    const { lit, waxing } = moonPhase(Date.UTC(y, m - 1, d, 20) - 8 * 3600000);
    const at = `${y}-${m}-${d} 农历${day} lit ${lit.toFixed(3)} ${waxing ? 'waxing' : 'waning'}`;
    if (day === 1 && !(lit < 0.08)) bad.push(at);
    if ((day === 15 || day === 16) && !(lit > 0.92)) bad.push(at);
    if (day >= 7 && day <= 9 && !(waxing && lit >= 0.2 && lit <= 0.8)) bad.push(at);
    if (day >= 22 && day <= 24 && !(!waxing && lit >= 0.2 && lit <= 0.8)) bad.push(at);
  }
  assert.deepEqual(bad, []);
});
