// node --test ui/tests/*.test.mts (Node strips the types).
// What Settings → Models takes from the engine's provider list.
import { test } from 'node:test';
import assert from 'node:assert/strict';
import { W } from '../src/lib/modelsWords.mts';
import { addedByName, capShare, chatChoices, dollars, spendMeter, editionWord, failureLabel, searchFamilies, splitFamilies, useLines, type Family, type Provider } from '../src/lib/providers.mts';

const family = (key: string, over: Partial<Family> = {}): Family => ({ family: key, label: key, ticked: false, choose: [], folded: false, references: [], ...over });
const provider = (over: Partial<Provider>): Provider => ({ id: 'p', label: 'Anthropic', way: 'key', provider: 'anthropic', url: '', connected: true, models: [], used_by: [], ...over });

test('a family row is named by what it runs, and an added model by its own name', () => {
  const p = provider({
    models: [
      { id: 'fam-1', model: 'claude-sonnet-5-5', kind: 'chat', ready: true, builtin: false },
      { id: 'own-1', model: 'claude-test', kind: 'chat', ready: true, builtin: false },
      { id: 'off', model: 'claude-off', kind: 'chat', ready: false, builtin: false },
    ],
    families: { catalog: { observed_at: '2026-11-23', age_days: 0, stale: false }, families: [family('claude-sonnet', { ticked: true, row: 'fam-1', current: { model: 'claude-sonnet-5-5', name: 'Claude Sonnet 5.5' } })] },
  });
  assert.deepEqual(chatChoices([p]).map((c) => [c.id, c.text]), [['fam-1', 'Claude Sonnet 5.5'], ['own-1', 'claude-test']]);
  assert.deepEqual(addedByName(p).map((m) => m.id), ['own-1', 'off']);
});

test('the other uses are the engine rows of each kind, ready or not', () => {
  const lanes = provider({ way: 'local', models: [{ id: 'speak', model: 'voice-x', kind: 'speak', ready: false, builtin: true }, { id: 'draw', model: 'pic-y', kind: 'draw', ready: true, builtin: true }] });
  assert.deepEqual(useLines([lanes]).map((u) => [u.word, u.model, u.ready]), [[W.readAloud, 'voice-x', false], [W.drawing, 'pic-y', true]]);
});

test('a live voice is a reference on its provider, never a default to pick', () => {
  const live = { id: 'live:alibaba', model: 'qwen-audio-3.1-realtime-flash', kind: 'live_voice' as const, ready: true, builtin: true };
  const p = provider({ models: [live], used_by: ['live_voice'] });
  assert.deepEqual(chatChoices([p]), []);
  assert.deepEqual(useLines([p]), []);
});

test('the region is said in the page\'s own words', () => {
  assert.equal(editionWord('international'), W.regionInternational);
  assert.equal(editionWord('mainland'), W.regionMainland);
});

test('families are split by the engine\'s fold, and a search matches the name or the key', () => {
  const view = { catalog: { observed_at: '', age_days: 0, stale: false }, families: [family('a'), family('b', { folded: true, label: 'Beta' }), family('c', { folded: true })] };
  const { shown, folded } = splitFamilies(view);
  assert.deepEqual([shown.length, folded.length], [1, 2]);
  assert.deepEqual(searchFamilies(folded, 'bet').map((f) => f.family), ['b']);
  assert.deepEqual(searchFamilies(folded, ' ').length, 2);
});

test('a route\'s fixed answer has its fixed label, and any other answer a general one', () => {
  assert.equal(failureLabel(409, 'first_default'), W.failFirstDefault);
  assert.equal(failureLabel(400, 'Invalid provider settings.'), W.failNotAccepted);
  assert.equal(failureLabel(403, ''), W.failOwnerOnly);
  assert.equal(failureLabel(500, 'a long raw error with a path /Users/x'), W.failOther);
});

test('a price is written without trailing zeros', () => {
  assert.equal(dollars(4), '4');
  assert.equal(dollars(0.1), '0.1');
  assert.equal(dollars(0.125), '0.125');
});

test('the cap meter says used of cap as an estimate, and what was used when there is no cap', () => {
  const s = { month: '2026-11', used_usd: 60.5, cap_usd: 200, prices_as_of: '2026-11-23' };
  assert.equal(spendMeter(s), 'Used about $60.5 of $200 this month (estimate)');
  assert.equal(capShare(s), 0.3025);
  assert.equal(capShare({ ...s, used_usd: 250 }), 1);
  const none = { month: '2026-11', used_usd: 3, prices_as_of: '2026-11-23' };
  assert.equal(spendMeter(none), 'Used about $3 this month (estimate)');
  assert.equal(capShare(none), 0);
});
