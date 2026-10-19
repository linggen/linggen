// node --test ui/tests/*.test.mts (Node strips the types).
// An app page mounted with params (a test save): every new chat through its
// embed carries them, and its list shows only sessions bound exactly so.
import { test } from 'node:test';
import assert from 'node:assert/strict';
import { boundTo, newChatRequest, parseBinding } from '../src/lib/sessionBinding.mts';

test('the embed reads its binding from ?params=, anything malformed is none', () => {
  assert.deepEqual(parseBinding('{"save":"x","n":3,"gone":""}'), { save: 'x', n: '3' });
  assert.deepEqual(parseBinding(null), {});
  assert.deepEqual(parseBinding('not json'), {});
  assert.deepEqual(parseBinding('["x"]'), {});
});

test('a new chat on a bound page carries the binding; unbound stays unbound', () => {
  assert.deepEqual(newChatRequest('t', 'lingjing', { save: 'x' }), { title: 't', skill: 'lingjing', params: { save: 'x' } });
  assert.deepEqual(newChatRequest('t', 'lingjing', {}), { title: 't', skill: 'lingjing' });
  assert.deepEqual(newChatRequest('t', null, { save: 'x' }), { title: 't' }, 'only a skill session is bound');
});

test('the list keeps only sessions bound exactly as the page', () => {
  const sessions = [
    { id: 'main' },
    { id: 'main-null', params: null },
    { id: 'x', params: { save: 'x' } },
    { id: 'y', params: { save: 'y' } },
    { id: 'x-more', params: { save: 'x', seat: '2' } },
  ];
  assert.deepEqual(boundTo(sessions, { save: 'x' }).map((s) => s.id), ['x']);
  assert.deepEqual(boundTo(sessions, {}).map((s) => s.id), ['main', 'main-null']);
});
