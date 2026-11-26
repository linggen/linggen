// node --test ui/tests/*.test.mts (Node strips the types).
// Linggen Cloud's use is shown as a percentage, never a count; the developer account says it is not metered.
import { test } from 'node:test';
import assert from 'node:assert/strict';
import { cloudUsage } from '../src/lib/cloudUsage.mts';

const signedIn = (extra: object) => ({ signed_in: true, entitlement: { usage: undefined, apps: {} }, ...extra });

test('a subscriber reads the percent the site returned', () => {
  const acc = signedIn({ entitlement: { usage: { used: 1, allowance: 10, warn: false, over: false, percent: 23, resets_at: '2026-12-01T00:00:00Z' } }, gate: { entitled: true } });
  assert.deepEqual(cloudUsage(acc as never), { text: 'Used this month: 23%', tone: 'ok', metered: true });
});

test('past 80% is a warning and a spent pool is spent', () => {
  const at = (percent: number, over = false) => signedIn({ entitlement: { usage: { used: 1, allowance: 10, warn: false, over, percent } }, gate: { entitled: true } });
  assert.equal(cloudUsage(at(85) as never)?.tone, 'warn');
  assert.equal(cloudUsage(at(100, true) as never)?.tone, 'spent');
});

test('a trial reads its percent', () => {
  const acc = signedIn({ gate: { entitled: false, trial: { tokens: 0, budget: 1, percent: 40, active: true } } });
  assert.deepEqual(cloudUsage(acc as never), { text: 'Trial used: 40%', tone: 'ok', metered: true });
});

test('the developer account says it is not metered, and signed out says nothing', () => {
  assert.deepEqual(cloudUsage(signedIn({ gate: { developer: true } }) as never), { text: 'Developer account, not metered', tone: 'ok', metered: false });
  assert.equal(cloudUsage({ signed_in: false }), null);
});

test('no token count or price is ever in the line', () => {
  const acc = signedIn({ entitlement: { usage: { used: 1_200_000, allowance: 10_000_000, warn: false, over: false, percent: 12 } }, gate: { entitled: true } });
  assert.doesNotMatch(cloudUsage(acc as never)!.text, /[0-9]M|\$|token/i);
});
