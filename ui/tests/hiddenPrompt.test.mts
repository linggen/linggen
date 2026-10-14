// node --test ui/tests/*.test.mts (Node strips the types).
// A prompt the engine wrote to an agent is never anyone's bubble; a person's
// own words always are, even said twice.
import { test } from 'node:test';
import assert from 'node:assert/strict';
import { shouldHideInternalChatMessage } from '../src/lib/hiddenRows.mts';

test("an engine kickoff ([HIDDEN]) is hidden, a person's words are not", () => {
  assert.equal(shouldHideInternalChatMessage('user', '[HIDDEN] A quiet moment — no event, just you.'), true);
  assert.equal(shouldHideInternalChatMessage('user', '[HIDDEN] [scene] opened'), true);
  assert.equal(shouldHideInternalChatMessage('user', 'A quiet moment, I wanted to say hi'), false);
  assert.equal(shouldHideInternalChatMessage('user', 'hello'), false);
});

test('streaming text that may still be SILENT is held back; real words are not', async () => {
  const { mayBecomeSilence, SILENT } = await import('../src/lib/hiddenRows.mts');
  assert.equal(SILENT, 'SILENT');
  for (const t of ['S', 'SIL', 'SILENT', ' SILENT.', 'silent']) assert.equal(mayBecomeSilence(t), true, t);
  for (const t of ['', 'Sure', 'Silent night', 'Good evening']) assert.equal(mayBecomeSilence(t), false, t);
});
