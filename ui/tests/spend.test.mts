// node --test ui/tests/*.test.mts (Node strips the types).
// What a session's top bar says about spend: one decision for every runtime.
import { test } from 'node:test';
import assert from 'node:assert/strict';
import { readFileSync } from 'node:fs';
import { translateFrom } from '../src/lib/i18nMatch.mts';
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

test('a partly priced session marks its amount and says what is not in it', () => {
  const model = (name: string, input: number, cost?: number) => ({
    model: name, billing: 'metered' as const, calls: 1, unreported: 0, input, output: 10, cache_read: 0, cache_write: 0,
    ...(cost === undefined ? {} : { cost_usd: cost }),
  });
  const view = spendView({
    claude: false, onChatGpt: false,
    usage: used({ cost_usd: 1, unpriced: ['gem'], models: [model('sonnet', 2000, 1), model('gem', 500)] }),
  });
  assert.equal(view.estimate?.text, '≈ $1.00+', 'the figure carries a visible mark');
  const title = view.estimate!.title;
  assert.match(title, /Not priced, so not in the amount: gem/);
  // The breakdown tells the priced tokens from the unpriced ones.
  assert.match(title, /priced models: 2\.0k in, 10 out, 0 cached/);
  assert.match(title, /gem \(500 in, 10 out, 0 cached\)/);
  // A fully priced session has no mark.
  assert.equal(spendView({ claude: false, onChatGpt: false, usage: used({ cost_usd: 1, models: [model('sonnet', 2000, 1)] }) }).estimate?.text, '≈ $1.00');
});

test('the Claude SDK session keeps its own figure, whatever the engine ledger says', () => {
  assert.deepEqual(spendView({ claude: true, claudeCost: 0.12, onChatGpt: false, usage: used({ cost_usd: 9 }) }), { claude: { cost: 0.12 } });
  assert.deepEqual(spendView({ claude: true, onChatGpt: false }), { claude: { cost: null } });
});

test('a ChatGPT-account chat shows its allowance; a session with no spend tokens shows no spend', () => {
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

// The string table is a .ts module Node does not load as one; its one table is read as data.
const ZH: Record<string, string> = new Function(
  readFileSync(new URL('../src/lib/strings-zh.ts', import.meta.url), 'utf8').replace(/^export const ZH[^=]*=/m, 'return '),
)();

test('every sentence the bar builds is put into Chinese by the page\'s own matcher, every slot filled', () => {
  const model = (name: string, input: number, cost?: number) => ({
    model: name, billing: 'metered' as const, calls: 1, unreported: 0, input, output: 10, cache_read: 3, cache_write: 0,
    ...(cost === undefined ? {} : { cost_usd: cost }),
  });
  const said = [
    spendView({ claude: false, onChatGpt: false, usage: used({ cost_usd: 4.4, models: [model('a', 1000, 4.4)] }) }).estimate!.title,
    spendView({ claude: false, onChatGpt: false, usage: used({ cost_usd: 1, unpriced: ['gem', 'zed'], models: [model('a', 2000, 1), model('gem', 500), model('zed', 70)] }) }).estimate!.title,
    spendView({ claude: false, onChatGpt: false, usage: used() }).tokens!.title,
    spendView({ claude: false, onChatGpt: false, usage: used() }).tokens!.text,
  ];
  for (const en of said) {
    const zh = translateFrom(ZH, en);
    assert.ok(zh, `no Chinese for: ${en}`);
    assert.ok(!/\{\w+\}/.test(zh!), `an unfilled slot in: ${zh}`);
    assert.ok(/[一-鿿]/.test(zh!), zh);
  }
  // The names and numbers of the English carry over.
  assert.match(translateFrom(ZH, said[1])!, /gem, zed（输入 576，输出 20，缓存命中 6）/);
});
