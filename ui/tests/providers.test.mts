// node --test ui/tests/*.test.mts (Node strips the types).
// What Settings → Models takes from the engine's provider list.
import { test } from 'node:test';
import assert from 'node:assert/strict';
import { W } from '../src/lib/modelsWords.mts';
import { chatChoices, chatModels, failureLabel, hasProblem, heldDefaults, isChatgpt, ownProviders, readyToUse, suggestionsAsk, suggestionsFor, type Provider, type ProviderModel } from '../src/lib/providers.mts';

const provider = (over: Partial<Provider>): Provider => ({ id: 'p', label: 'Anthropic', way: 'key', provider: 'anthropic', url: '', connected: true, models: [], used_by: [], ...over });
const model = (id: string, over: Partial<ProviderModel> = {}): ProviderModel => ({ id, model: id, name: id, kind: 'chat', ready: true, builtin: false, ...over });

test('a chat choice is named as the engine names it, ready ones only', () => {
  const p = provider({ models: [model('cloud-1', { name: 'Qwen 3.8 Flash' }), model('own-1'), model('off', { ready: false })] });
  assert.deepEqual(chatChoices([p]).map((c) => [c.id, c.text]), [['cloud-1', 'Qwen 3.8 Flash'], ['own-1', 'own-1']]);
  assert.deepEqual(chatModels(p).map((m) => m.id), ['cloud-1', 'own-1', 'off']);
});

test('a live voice is a reference on its provider, never a default to pick', () => {
  const p = provider({ models: [model('live:alibaba', { model: 'qwen-audio-3.1-realtime-flash', kind: 'live_voice', builtin: true })], used_by: ['live_voice'] });
  assert.deepEqual(chatChoices([p]), []);
});

test('typing suggests the provider\'s own ids that match, hides nothing else, and leaves out what is added', () => {
  const list = [{ id: 'gpt-terra' }, { id: 'tts-1' }, { id: 'text-embedding-3' }, { id: 'claude-x', name: 'Claude Test' }];
  assert.deepEqual(suggestionsFor(list, 'te', []).map((s) => s.id), ['gpt-terra', 'text-embedding-3', 'claude-x']);
  assert.deepEqual(suggestionsFor(list, 'te', ['gpt-terra']).map((s) => s.id), ['text-embedding-3', 'claude-x']);
  assert.deepEqual(suggestionsFor(list, ' ', []), []);
  assert.deepEqual(suggestionsFor(null, 'te', []), [], 'no list: nothing suggested; Enter still adds');
  assert.equal(suggestionsFor(list, 't', [], 2).length, 2);
});

test('the remove confirm names the default and the fallbacks a provider holds', () => {
  const p = provider({ models: [model('a'), model('b')] });
  assert.deepEqual(heldDefaults(p, ['a', 'x', 'b']), { first: true, fallbacks: ['b'] });
  assert.deepEqual(heldDefaults(p, ['x', 'y']), { first: false, fallbacks: [] });
  assert.deepEqual(heldDefaults(p, []), { first: false, fallbacks: [] });
});

test('a route\'s fixed answer has its fixed label, and any other answer a general one', () => {
  assert.equal(failureLabel(409, 'first_default'), W.failFirstDefault);
  assert.equal(failureLabel(400, 'Invalid provider settings.'), W.failNotAccepted);
  assert.equal(failureLabel(403, ''), W.failOwnerOnly);
  assert.equal(failureLabel(500, 'a long raw error with a path /Users/x'), W.failOther);
});

test('a provider has a problem when its key clashes or is set but not working', () => {
  assert.equal(hasProblem(provider({ key: { set: true }, connected: false })), true);
  assert.equal(hasProblem(provider({ key: { set: true, conflict: true } })), true);
  assert.equal(hasProblem(provider({ key: { set: true } })), false);
});

test('only Linggen Cloud stays in the top block; the ChatGPT sign-in is an own provider with ticks', () => {
  const base = { url: '', connected: true, models: [], used_by: [] };
  const cloud = { ...base, id: 'c', label: 'Linggen Cloud', way: 'account', provider: 'openai' } as Provider;
  const chatgpt = { ...base, id: 'g', label: 'ChatGPT 订阅', way: 'subscription', provider: 'chatgpt', chatgpt: true } as Provider;
  const key = { ...base, id: 'k', label: 'DeepSeek', way: 'key', provider: 'deepseek' } as Provider;
  assert.deepEqual(readyToUse([cloud, chatgpt, key]).map((p) => p.id), ['c']);
  assert.deepEqual(ownProviders([cloud, chatgpt, key]).map((p) => p.id), ['g', 'k']);
  assert.equal(isChatgpt(chatgpt), true);
  assert.equal(isChatgpt(key), false);
});

test('a key row says its models are for chat and has no live-voice line; Ollama and ChatGPT carry no such note', async () => {
  const { readFileSync } = await import('node:fs');
  const src = readFileSync(new URL('../src/components/models/KeyRow.tsx', import.meta.url), 'utf8');
  assert.doesNotMatch(src, /live-model|liveModels/);
  assert.match(src, /!isOllama\(p\) && !isChatgpt\(p\) && <p[^>]*chat-models-note[^>]*>\{W\.chatModelsNote\}/);
});

test('the ChatGPT row shows the shared model list and input, with no key field', async () => {
  const { readFileSync } = await import('node:fs');
  const row = readFileSync(new URL('../src/components/models/ChatgptRow.tsx', import.meta.url), 'utf8');
  assert.match(row, /<Models provider=\{p\}/);
  assert.doesNotMatch(row, /KeyField|ModelInput|tickChatgpt/);
  const keys = readFileSync(new URL('../src/components/models/KeyRow.tsx', import.meta.url), 'utf8');
  assert.match(keys, /isChatgpt\(p\)\) && <ModelInput/);
});

test('the list is asked again when the key or the address changes, and not otherwise', () => {
  const p = { id: 'k1', url: 'https://a.example/v1' } as Provider;
  assert.equal(suggestionsAsk(p, 0), suggestionsAsk({ ...p }, 0));
  assert.notEqual(suggestionsAsk(p, 0), suggestionsAsk(p, 1));
  assert.notEqual(suggestionsAsk(p, 0), suggestionsAsk({ ...p, url: 'https://b.example/v1' }, 0));
});
