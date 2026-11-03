// node --test ui/tests/*.test.mts (Node strips the types).
// A call waits as long as it declares; a slow reply never reads as a dead
// link — only an unanswered heartbeat does, and a throttled tab is not one.
import { test } from 'node:test';
import assert from 'node:assert/strict';
import {
  budgetMs, createLiveness, DEFAULT_BUDGET_MS, MAX_BUDGET_MS, PING_EVERY_MS, PONG_WAIT_MS, REPLY_MARGIN_MS,
} from '../src/lib/requestBudget.mts';

test('a call that declares nothing waits the default', () => {
  assert.equal(budgetMs(undefined), DEFAULT_BUDGET_MS);
  assert.equal(budgetMs(null, { command: 'ls' }), DEFAULT_BUDGET_MS);
  assert.equal(budgetMs('abc', 'text body'), DEFAULT_BUDGET_MS);
});

test('an explicit budget wins, from a number or a header string', () => {
  assert.equal(budgetMs(255_000, { timeout_ms: 10 }), 255_000);
  assert.equal(budgetMs('255000'), 255_000);
});

test("a body's timeout_ms is the work it asks for, plus time for the reply", () => {
  assert.equal(budgetMs(undefined, { timeout_ms: 240_000 }), 240_000 + REPLY_MARGIN_MS);
});

test('a budget is never under the default nor over the cap', () => {
  assert.equal(budgetMs(5_000), DEFAULT_BUDGET_MS);
  assert.equal(budgetMs(Number.MAX_SAFE_INTEGER), MAX_BUDGET_MS);
  assert.equal(budgetMs(Infinity), DEFAULT_BUDGET_MS);
});

test('a ping answered in time keeps the link alive', () => {
  const live = createLiveness();
  live.pinged(0);
  live.heard();
  assert.equal(live.dead(PONG_WAIT_MS + 1), false);
});

test('a ping unanswered past the wait means the link is dead', () => {
  const live = createLiveness();
  live.pinged(0);
  live.pinged(PING_EVERY_MS); // a second ping does not restart the clock
  assert.equal(live.dead(PONG_WAIT_MS), false);
  assert.equal(live.dead(PONG_WAIT_MS + 1), true);
});

test('a throttled tab (timers once a minute) is not read as dead', () => {
  const live = createLiveness();
  // Tick at 0: ping. The answer lands at once; the next tick comes 60 s later.
  live.pinged(0);
  live.heard();
  assert.equal(live.dead(60_000), false);
});

test('nothing pinged yet is never dead', () => {
  assert.equal(createLiveness().dead(10 ** 9), false);
});
