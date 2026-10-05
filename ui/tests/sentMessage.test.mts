// node --test ui/tests/*.test.mts (Node strips the types).
// A person's bubble and the row the engine kept for it match by the id the
// bubble was sent with — the row keeps only the words after `@name`.
import { test } from 'node:test';
import assert from 'node:assert/strict';
import { confirmSent, newClientId, sameSentMessage } from '../src/lib/sentMessage.mts';
import type { ChatMessage } from '../src/types.ts';

const bubble = (text: string, clientId?: string): ChatMessage => ({
  role: 'user', from: 'user', to: 'yinyue', text, timestamp: '', timestampMs: 1_000,
  ...(clientId ? { clientId } : {}),
});

test('each bubble gets its own short id', () => {
  const a = newClientId();
  const b = newClientId();
  assert.notEqual(a, b);
  assert.match(a, /^[A-Za-z0-9_-]{1,64}$/);
});

test('the kept row matches its bubble by id, whatever the words', () => {
  const typed = bubble('@银月 wish me luck', 'c-1');
  const kept = bubble('wish me luck', 'c-1');
  assert.equal(sameSentMessage(kept, typed), true);
  assert.equal(sameSentMessage(kept, bubble('wish me luck', 'c-2')), false);
  // No id on either side: never the same by id (words decide elsewhere).
  assert.equal(sameSentMessage(bubble('hi'), bubble('hi')), false);
});

test('the echoed event turns the bubble into what was kept, no second row', () => {
  const msgs = [bubble('earlier', 'c-0'), bubble('@银月 wish me luck', 'c-1')];
  const next = confirmSent(msgs, 'c-1', 'wish me luck', 'yinyue');
  assert.ok(next);
  assert.equal(next.length, 2);
  assert.equal(next[1].text, 'wish me luck');
  assert.equal(next[1].clientId, 'c-1');
  assert.equal(next[1].timestampMs, 1_000);
  assert.equal(msgs[1].text, '@银月 wish me luck', 'the old list is untouched');
});

test('a message another surface sent is not this one’s bubble', () => {
  assert.equal(confirmSent([bubble('hi', 'c-1')], 'c-9', 'hi', 'ling'), null);
});
