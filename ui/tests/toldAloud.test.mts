// A notice of Ling's work is never a row: the thing it is about carries its
// folded mark (lib/toldAloud.mts), and her bubble, telling of a wait, carries
// the waiting question's own card — the one implementation of its buttons
// (components/PendingAskCard.tsx), the lock below.
import assert from 'node:assert/strict';
import { readFileSync, readdirSync, statSync } from 'node:fs';
import { join, relative } from 'node:path';
import { test } from 'node:test';
import { fileURLToPath } from 'node:url';
import { markRun, toldLabel, toldOf, toldOfEvent } from '../src/lib/toldAloud.mts';
import { isPermissionAsk, listedAsk, markAsk } from '../src/lib/askQueue.mts';
import { speechAbout } from '../src/lib/petCue.mts';

test('a told reply marks its run\'s last row, and nothing else', () => {
  const told = { atMs: 1_000, line: 'Your shelf plan is ready.' };
  const rows = [{ runId: 'a', text: 'call' }, { runId: 'a', text: 'reply' }, { runId: 'b', text: 'other' }, { text: 'mine' }];
  const marked = markRun(rows, 'a', told);
  assert.equal(marked[0].toldAloud, undefined);
  assert.deepEqual(marked[1].toldAloud, told);
  assert.equal(marked[2].toldAloud, undefined);
  assert.equal(markRun(rows, 'missing', told), rows, 'no run, unchanged');
});

test('the mark is read from a row, a listed question and the live event', () => {
  assert.deepEqual(toldOf({ told_aloud_at: 60, told_aloud_line: 'Ready.' }), { atMs: 60_000, line: 'Ready.' });
  assert.equal(toldOf({}), undefined);
  assert.deepEqual(toldOfEvent({ at: 2, line: 'x' }), { atMs: 2_000, line: 'x' });
  const ask = listedAsk({ question_id: 'q', questions: [], told_aloud_at: 3, told_aloud_line: 'They need you.' });
  assert.deepEqual(ask.toldAloud, { atMs: 3_000, line: 'They need you.' });
  assert.equal(listedAsk({ question_id: 'q' }).toldAloud, undefined, 'never told, never marked');
  assert.match(toldLabel({ atMs: new Date(2026, 10, 20, 18, 13).getTime(), line: '' }), /while you were away · 18:13$/);
});

test('a live mark lands on its waiting card in its session only', () => {
  const ask = (questionId: string) => ({ questionId, agentId: 'ling', questions: [] });
  const queues = { s: [ask('1'), ask('2')], t: [ask('1')] };
  const told = { atMs: 5, line: 'l' };
  const marked = markAsk(queues, 's', '2', told);
  assert.deepEqual(marked.s[1].toldAloud, told);
  assert.equal(marked.s[0].toldAloud, undefined);
  assert.equal(marked.t[0].toldAloud, undefined);
  assert.equal(markAsk(queues, 's', 'gone', told), queues);
});

test('a cue that tells of a wait carries its question for the bubble\'s card', () => {
  assert.deepEqual(speechAbout({ question_id: 'q-1' }), { questionId: 'q-1' });
  assert.deepEqual(speechAbout({ problem: 'p', offers: [1] }), { raised: { problem: 'p', offers: [1] } });
  assert.equal(speechAbout({ problem: null, offers: [], question_id: null }), undefined);
  const permission = { questionId: 'q', agentId: 'ling', questions: [{ question: 'Bash ls', header: 'Permission', options: [], multi_select: false }] };
  assert.ok(isPermissionAsk(permission as never));
});

const root = fileURLToPath(new URL('../src/', import.meta.url));
function files(dir: string): string[] {
  return readdirSync(dir).flatMap((name) => {
    const path = join(dir, name);
    return statSync(path).isDirectory() ? files(path) : /\.(tsx?|mts)$/.test(name) ? [path] : [];
  });
}

test('a waiting question\'s buttons are drawn by one card, in every place it shows', () => {
  const drawn = files(root)
    .filter((path) => /<(ToolPermissionCard|AskUserCard)\b/.test(readFileSync(path, 'utf8')))
    .map((path) => relative(root, path));
  assert.deepEqual(drawn, ['components/PendingAskCard.tsx']);
  const bubble = readFileSync(join(root, 'components/YinyueBubble.tsx'), 'utf8');
  assert.match(bubble, /<PendingAskCard\b/, 'her bubble carries the waiting card');
});
