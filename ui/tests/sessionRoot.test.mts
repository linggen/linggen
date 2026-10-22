// node --test ui/tests/*.test.mts (Node strips the types).
// A folder picked for one session never reaches the next New chat: the
// create request carries no folder, and the made session's own folder
// replaces the previous chat's.
import { test } from 'node:test';
import assert from 'node:assert/strict';
import { afterNewChat, sessionRootOf } from '../src/lib/sessionRoot.mts';
import { newChatRequest } from '../src/lib/sessionBinding.mts';

test('a plain New chat asks for no folder', () => {
  const req = newChatRequest('Chat Oct 22, 10:51 AM');
  assert.deepEqual(req, { title: 'Chat Oct 22, 10:51 AM' });
  assert.equal('project_root' in req, false);
});

test("a stale picked project is replaced by the made session's folder", () => {
  const before = [{ id: 'old', repo_path: '', title: 'x', created_at: 1, project: '/work/linggen' }];
  const after = afterNewChat(before, { id: 'new', title: 'Chat', cwd: '/home/.linggen/workspace' }, 2);
  assert.equal(after.activeSessionId, 'new');
  assert.equal(after.selectedProjectRoot, '/home/.linggen/workspace');
  assert.equal(after.allSessions[0].id, 'new');
  assert.equal(sessionRootOf(after.allSessions[0] as { id: string; cwd: string }), '/home/.linggen/workspace');
  assert.equal(after.allSessions.length, 2);
});

test('an engine that sent no folder leaves none, not the old one', () => {
  const after = afterNewChat([], { id: 'new', title: 'Chat' }, 2);
  assert.equal(after.selectedProjectRoot, '');
});

test("a session's folder is its project, else its cwd", () => {
  assert.equal(sessionRootOf({ id: 'a', project: '/p', cwd: '/p/src' }), '/p');
  assert.equal(sessionRootOf({ id: 'a', cwd: '/c' }), '/c');
  assert.equal(sessionRootOf(null), '');
});
