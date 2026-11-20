// node --test ui/tests/*.test.mts (Node strips the types).
// What a session's top bar says about spend: one decision for every runtime.
import { test } from 'node:test';
import assert from 'node:assert/strict';
import { readFileSync } from 'node:fs';
import { compactTokens, dollars, spendView, type SessionSpend } from '../src/lib/spend.mts';
import { loadOutcome } from '../src/lib/sessionLoad.mts';

const used = (over: Partial<SessionSpend> = {}): SessionSpend => ({
  models: [],
  tokens: { input: 1_200_000, output: 100_000, cache_read: 5_000_000, cache_write: 300_000 },
  ...over,
});

test('a priced model shows an estimate, with the tokens behind it on hover', () => {
  const view = spendView({ claude: false, onChatGpt: false, usage: used({ cost_usd: 4.4 }) });
  assert.equal(view.estimate?.text, '≈ $4.40');
  assert.equal(view.tokens, undefined);
  // in = fresh + cache read + cache write; cached = cache read.
  assert.match(view.estimate!.title, /6\.5M in, 100\.0k out, 5\.0M cached/);
  assert.match(view.estimate!.title, /Your bill may differ/);
});

test('an unpriced model shows tokens and no dollars', () => {
  const view = spendView({ claude: false, onChatGpt: false, usage: used() });
  assert.equal(view.estimate, undefined);
  assert.equal(view.tokens?.text, '6.5M in · 100.0k out');
  assert.ok(!/\$/.test(view.tokens!.text + view.tokens!.title), 'no amount is invented');
});

test('a priced model beside an unpriced one says the amount leaves tokens out', () => {
  const view = spendView({ claude: false, onChatGpt: false, usage: used({ cost_usd: 1, unpriced: ['gem'] }) });
  assert.match(view.estimate!.title, /No price is known for gem: those tokens are not in the amount\./);
});

test('the Claude SDK session keeps its own figure, whatever the engine ledger says', () => {
  assert.deepEqual(spendView({ claude: true, claudeCost: 0.12, onChatGpt: false, usage: used({ cost_usd: 9 }) }), { claude: { cost: 0.12 } });
  assert.deepEqual(spendView({ claude: true, onChatGpt: false }), { claude: { cost: null } });
});

test('a ChatGPT-account chat shows its allowance; account models add no spend', () => {
  assert.deepEqual(spendView({ claude: false, onChatGpt: true, usage: null }), { allowance: true });
  const none = { input: 0, output: 0, cache_read: 0, cache_write: 0 };
  assert.deepEqual(spendView({ claude: false, onChatGpt: false, usage: used({ tokens: none }) }), {});
  assert.deepEqual(spendView({ claude: false, onChatGpt: false }), {});
});

test('numbers read short', () => {
  assert.equal(compactTokens(38_100_000), '38.1M');
  assert.equal(compactTokens(900), '900');
  assert.equal(dollars(4.4), '4.40');
  assert.equal(dollars(0.0144), '0.014');
});

test('a load that changed only the spend is applied', () => {
  const before = { sessionId: 's', messageCount: 2, spend: 10 };
  assert.equal(loadOutcome({ ...before, spend: 10 }, 's', before), 'unchanged');
  assert.equal(loadOutcome({ ...before, spend: 99 }, 's', before), 'apply');
});

test('every sentence the bar can say has its Chinese in the string table', () => {
  const zh = readFileSync(new URL('../src/lib/strings-zh.ts', import.meta.url), 'utf8');
  for (const key of [
    "Estimate at list price from this session's tokens: {a} in, {b} out, {c} cached. Your bill may differ.",
    "Estimate at list price from this session's tokens: {a} in, {b} out, {c} cached. Your bill may differ. No price is known for {names}: those tokens are not in the amount.",
    'Tokens used: {a} in, {b} out, {c} cached. No price is known for this model, so no cost is shown.',
    '{a} in · {b} out',
  ]) assert.ok(zh.includes(`'${key}'`) || zh.includes(`"${key}"`), key);
  // The longer sentence is tried first: the shorter one is its prefix.
  assert.ok(zh.indexOf('No price is known for {names}') < zh.indexOf('differ.": '));
});
