// node --test ui/tests/*.test.mts (Node strips the types).
// The spinner under the chat says what state the agent is in, never which
// tool runs: the tool row in the chat shows that.
import { test } from 'node:test';
import assert from 'node:assert/strict';
import { activeFormLabel, runLabel, sessionStatusText, spinnerLabel, WORKING } from '../src/lib/runStatus.mts';

test('a running tool reads as Working, whatever the event says of it', () => {
  assert.equal(sessionStatusText('calling_tool', 'Running command: cargo build'), WORKING);
  assert.equal(sessionStatusText('calling_tool', 'Reading file: src/main.rs'), WORKING);
  assert.equal(sessionStatusText('calling_tool', ''), WORKING);
});

test('the other states keep the engine words', () => {
  assert.equal(sessionStatusText('thinking', 'Thinking (some-model)'), 'Thinking (some-model)');
  assert.equal(sessionStatusText('thinking', 'Thinking (compacting context)'), 'Thinking (compacting context)');
  assert.equal(sessionStatusText('model_loading', 'Loading model: some-model'), 'Loading model: some-model');
  assert.equal(sessionStatusText('working', 'Running'), 'Running');
});

test('without words, the state itself', () => {
  assert.equal(sessionStatusText('thinking', ''), 'Thinking');
  assert.equal(sessionStatusText('model_loading', ''), 'Model Loading');
  assert.equal(sessionStatusText('working', ''), WORKING);
  assert.equal(sessionStatusText('idle', 'anything'), 'Idle');
});

test('a turning spinner never says Idle: the last turn ended, this one has no status yet', () => {
  assert.equal(spinnerLabel('Idle'), undefined);
  assert.equal(spinnerLabel(undefined), undefined);
  assert.equal(spinnerLabel(WORKING), WORKING);
  assert.equal(spinnerLabel('Thinking (model)'), 'Thinking (model)');
});

const working = (status: string) => ({
  status,
  items: [
    { status: 'completed', active_form: 'Reading code' },
    { status: 'in_progress', active_form: 'Running tests' },
    { status: 'pending', active_form: 'Writing docs' },
  ],
});

test('while a plan is worked, the line says what the in-progress todo is doing', () => {
  assert.equal(runLabel(working('executing'), 'Working', 'Musing'), 'Running tests');
  assert.equal(runLabel(working('approved'), undefined, 'Musing'), 'Running tests');
});

test('a plan not being worked, or a todo without an active form, leaves the old label', () => {
  for (const status of ['planned', 'completed', 'rejected']) {
    assert.equal(activeFormLabel(working(status)), undefined, status);
  }
  assert.equal(runLabel(working('completed'), 'Working', 'Musing'), 'Working');
  const bare = { status: 'executing', items: [{ status: 'in_progress' }] };
  assert.equal(runLabel(bare, 'Idle', 'Musing'), 'Musing');
  assert.equal(runLabel(null, undefined, ''), 'Thinking');
});
