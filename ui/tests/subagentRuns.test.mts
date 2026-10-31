// node --test ui/tests/*.test.mts (Node strips the types).
// A native session's subagent rows settle against the engine's runs: a row
// whose run ended while its end event was lost stops showing "running", and a
// child the page never saw start gets its row (2026-10-31: nine finished
// subagents showed "running 16m" after the channel dropped).
import { test } from 'node:test';
import assert from 'node:assert/strict';
import { SETTLE_GRACE_MS, rowForRun, settleTree, unshownChildren } from '../src/lib/subagentRuns.mts';

const row = (subagentId: string, status: 'running' | 'done' | 'failed', timestampMs?: number) => ({
  subagentId, agentName: 'ling', task: 'look', status, toolCount: 0, contextTokens: 0,
  currentActivity: 'Grep', toolSteps: [], ...(timestampMs === undefined ? {} : { timestampMs }),
});
const run = (run_id: string, parent_run_id: string | null, status = 'running') => ({
  run_id, parent_run_id, status, repo_path: '', session_id: 's', agent_id: 'ling', detail: `task of ${run_id}`,
  started_at: 100, ended_at: null,
}) as any;

test('a running row whose run ended settles; live, young and ended rows stay', () => {
  const now = 1_000_000;
  const tree = [row('gone', 'running', 0), row('live', 'running', 0), row('young', 'running', now - 1000),
    row('failed', 'failed', 0), row('saved', 'running')];
  const settled = settleTree(tree, new Set(['live']), now);
  assert.deepEqual(settled.map((e) => e.status), ['done', 'running', 'running', 'failed', 'done']);
  assert.equal(settled[0].currentActivity, null);
  assert.ok(now - (now - 1000) < SETTLE_GRACE_MS);
});

test('nothing to settle keeps the same tree', () => {
  const tree = [row('live', 'running', 0)];
  assert.equal(settleTree(tree, new Set(['live']), 1_000_000), tree);
});

test('a running child no row shows gets one; top runs and shown ones do not', () => {
  const runs = [run('top', null), run('child', 'top'), run('shown', 'top'), run('ended', 'top', 'completed')];
  const missing = unshownChildren(runs, new Set(['shown']));
  assert.deepEqual(missing.map((r) => r.run_id), ['child']);
  const entry = rowForRun(missing[0]);
  assert.equal(entry.subagentId, 'child');
  assert.equal(entry.task, 'task of child');
  assert.equal(entry.status, 'running');
  assert.equal(entry.timestampMs, 100_000);
});
