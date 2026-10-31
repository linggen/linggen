import { test } from 'node:test';
import assert from 'node:assert/strict';
import { noteServed, hasTypedText } from '../src/lib/uiBuild.mts';

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
