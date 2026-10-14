// node --test ui/tests/*.test.mts (Node strips the types).
// Words said twice are two rows; only one row delivered twice collapses.
import { test } from 'node:test';
import assert from 'node:assert/strict';
import { dropLiveEchoes, isEventEcho, isLiveEcho } from '../src/lib/rowIdentity.mts';
import type { ChatMessage } from '../src/types.ts';

const T = 1_791_984_822_000;
const said = (text: string, ts: number, clientId?: string): ChatMessage => ({
  role: 'user', from: 'user', to: 'ling', text, timestamp: '', timestampMs: ts,
  ...(clientId ? { clientId } : {}),
});
const reply = (from: string, text: string, ts: number, extra: Partial<ChatMessage> = {}): ChatMessage => ({
  role: 'agent', from, to: 'user', text, timestamp: '', timestampMs: ts, ...extra,
});

test('the same words with two sent ids are two rows, wherever they come from', () => {
  const first = said('restart game', T, 'c-mv85huab-1kviy0g3');
  const second = said('restart game', T + 25 * 60_000, 'c-mv86edbn-78kjebw7');
  // A history refresh: the second arrives live, the first is saved.
  assert.deepEqual(dropLiveEchoes([first], [second]), [first, second]);
  // Both saved, plus this panel's own bubble of the second.
  const bubble = { ...second, timestampMs: second.timestampMs! - 400 };
  assert.deepEqual(dropLiveEchoes([first, second], [bubble]), [first, second]);
  // A live `message` event for the second, the first already on screen.
  assert.equal(isEventEcho([first, reply('ling', 'New game.', T + 3_000)], second), false);
  // Even seconds apart: an id decides, never the words.
  assert.equal(isLiveEcho(said('ok', T + 1_000, 'c-b'), said('ok', T, 'c-a')), false);
});

test('one sent message delivered twice shows once', () => {
  const row = said('continue', T, 'c-1');
  assert.equal(isEventEcho([row], { ...row, timestampMs: T + 700 }), true);
  assert.deepEqual(dropLiveEchoes([row], [{ ...row }]), [row]);
});

test('a row without an id, replayed at the same moment, shows once; said later, it shows again', () => {
  const ok = said('ok', T);
  assert.equal(isEventEcho([ok], said('ok', T + 400)), true);
  assert.equal(isEventEcho([ok], said('ok', T + 90_000)), false);
  assert.deepEqual(dropLiveEchoes([ok], [said('ok', T + 90_000)]), [ok, said('ok', T + 90_000)]);
});

test('the original double delivery: a finalized live reply and its saved row stay one', () => {
  // The live bubble is stamped when the turn began (run id as sender); the
  // saved row when it finished, minutes later, past every merge pass.
  const live = reply('run-ling-1791984822', 'The board is reset.', T, { segments: [] });
  const saved = reply('ling', 'The board is reset.', T + 3 * 60_000);
  assert.deepEqual(dropLiveEchoes([saved], [live]), [saved]);
  // An earlier saved reply with the same words is a repeat, not this row.
  const earlier = reply('ling', 'The board is reset.', T - 10 * 60_000);
  assert.equal(dropLiveEchoes([earlier], [live]).length, 2);
});

test('turn_complete finalized the reply before its message event: no second row', () => {
  const asked = said('restart game', T - 1_000, 'c-1');
  const bubble = reply('ling', 'New game.', T, { durationMs: 90_000 });
  const event = reply('ling', 'New game.', T + 90_000);
  assert.equal(isEventEcho([asked, bubble], event), true);
  // Asked again since: the same answer is a new row.
  const askedAgain = said('restart game', T + 100_000, 'c-2');
  assert.equal(isEventEcho([asked, bubble, askedAgain], { ...event, timestampMs: T + 120_000 }), false);
});

test('a reply still generating always shows', () => {
  const saved = reply('ling', 'Thinking', T);
  const live = reply('ling', 'Thinking', T, { isGenerating: true });
  assert.equal(dropLiveEchoes([saved], [live]).length, 2);
});
