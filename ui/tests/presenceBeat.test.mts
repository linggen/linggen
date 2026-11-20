// node --test ui/tests/*.test.mts (Node strips the types).
// A covered surface's "no longer here" counts as told only when the engine
// took it. Found by review (2026-11-20): the beat recorded the withdrawal
// before sending it, so one failed request left the engine sure the person
// was still at the chat while the app tab covered it.
import { test } from 'node:test';
import assert from 'node:assert/strict';
import { presenceBeat } from '../src/lib/presenceBeat.mts';

function surface(answers: Array<boolean | 'throw'>) {
  const state = { covered: false, focused: true, sent: [] as boolean[] };
  const beat = presenceBeat({
    isOpen: () => true,
    covered: () => state.covered,
    read: () => ({ focused: !state.covered && state.focused }),
    send: async (reading) => {
      state.sent.push(reading.focused);
      const answer = answers.shift() ?? true;
      if (answer === 'throw') throw new Error('offline');
      return answer;
    },
  });
  return { state, beat };
}

test('a withdrawal the engine did not take is sent again on the next beat', async () => {
  const { state, beat } = surface([true, false, true]);
  await beat(); // focused, taken
  state.covered = true;
  await beat(); // "not here" — the request fails
  await beat(); // the next interval tries again, and it is taken
  await beat(); // told: a covered surface stays quiet
  assert.deepEqual(state.sent, [true, false, false]);
});

test('a withdrawal whose request throws is sent again too', async () => {
  const { state, beat } = surface([true, 'throw', true]);
  await beat();
  state.covered = true;
  await beat();
  await beat();
  await beat();
  assert.deepEqual(state.sent, [true, false, false]);
});

test('a covered surface the engine never took as focused does not beat', async () => {
  const { state, beat } = surface([false]);
  await beat(); // focused, but the engine did not take it
  state.covered = true;
  await beat();
  assert.deepEqual(state.sent, [true]);
});
