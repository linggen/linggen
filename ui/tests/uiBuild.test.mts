import { test } from 'node:test';
import assert from 'node:assert/strict';
import { noteServed, hasTypedText, idleForReload, TEXT_FIELDS } from '../src/lib/uiBuild.mts';

test('the first build a page sees is the one it loaded; a different one makes it stale for good', () => {
  let s = noteServed({ loaded: null, stale: false }, undefined);
  assert.deepEqual(s, { loaded: null, stale: false }, 'a push without the build says nothing');
  s = noteServed(s, 'aaa');
  assert.deepEqual(s, { loaded: 'aaa', stale: false });
  s = noteServed(s, 'aaa');
  assert.equal(s.stale, false);
  s = noteServed(s, 'bbb');
  assert.equal(s.stale, true);
  s = noteServed(s, 'aaa');
  assert.equal(s.stale, true, 'the page still runs the bundle it loaded');
});

test('text typed in any field holds the reload back', () => {
  assert.equal(hasTypedText([{ value: '' }, { value: '  ' }]), false);
  assert.equal(hasTypedText([{ value: '' }, { value: 'draft' }]), true);
  assert.equal(hasTypedText([{ isContentEditable: true, textContent: 'note' }]), true);
});

test('an out-of-date page reloads only when nothing would be cut short or lost', () => {
  const idle = { replying: false, awaitingAnswer: false, dialogOpen: false };
  assert.equal(idleForReload(idle, false), true);
  assert.equal(idleForReload({ ...idle, replying: true }, false), false, 'a reply is being written');
  assert.equal(idleForReload({ ...idle, awaitingAnswer: true }, false), false, 'an ask or plan card waits');
  assert.equal(idleForReload({ ...idle, dialogOpen: true }, false), false, 'a dialog is open');
  assert.equal(idleForReload(idle, true), false, 'something is typed');
});

test('every text-like field counts as a place to type', () => {
  for (const sel of ['textarea', 'input:not([type])', 'input[type="search"]', 'input[type="email"]',
    'input[type="number"]', 'input[type="url"]', 'input[type="tel"]', 'input[type="password"]', '[contenteditable]']) {
    assert.ok(TEXT_FIELDS.includes(sel), sel);
  }
});
