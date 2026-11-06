import { test } from 'node:test';
import assert from 'node:assert/strict';
import { isRestartCommand, requestRestart, restartSuggestion } from '../src/lib/restartCommand.mts';

test('only the standalone command without attachments is intercepted', () => {
  assert.equal(isRestartCommand('/restart'), true);
  assert.equal(isRestartCommand('  /restart\n'), true);
  for (const text of ['/restart now', '/restart-engine', '/Restart', '@ling /restart', 'please /restart']) {
    assert.equal(isRestartCommand(text), false, text);
  }
  assert.equal(isRestartCommand('/restart', ['image']), false);
});

test('the shared suggestion names the exact command', () => {
  assert.equal(restartSuggestion.label, '/restart');
});

test('restart posts to the current-engine endpoint and propagates failure', async () => {
  const calls: string[] = [];
  await requestRestart(async (path) => { calls.push(path); });
  assert.deepEqual(calls, ['/api/restart/current']);
  await assert.rejects(requestRestart(async () => { throw new Error('denied'); }), /denied/);
});
