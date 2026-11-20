import assert from 'node:assert/strict';
import { test } from 'node:test';
import { isLauncherWindow, resolveLauncherFace } from '../src/lib/launcherFace.mts';

test('the launcher entry remembers the face until explicitly cleared', () => {
  let remembered = false;
  for (const [search, expected] of [['', false], ['?launcher=1', true], ['', true], ['?launcher=0', false], ['', false]] as const) {
    const face = resolveLauncherFace(search, true, remembered);
    assert.equal(face.launcher, expected);
    remembered = face.remember;
  }
});

test('frames, pet, and channel do not take or change the launcher face', () => {
  for (const search of ['', '?mode=compact', '?channel=1', '?pet=1', '?launcher=0', '?launcher=1']) {
    for (const remembered of [false, true]) {
      assert.deepEqual(resolveLauncherFace(search, false, remembered), { launcher: false, remember: remembered });
    }
  }
  for (const search of ['?pet=1&launcher=0', '?channel=1&launcher=0']) {
    assert.deepEqual(resolveLauncherFace(search, true, true), { launcher: false, remember: true });
  }
  const frame = { top: {}, location: { search: '' }, get sessionStorage() { assert.fail('frame read the window mark'); } };
  assert.equal(isLauncherWindow(frame as unknown as Window), false);
});

test('a named face beats the remembered launcher but not an explicit launcher entry', () => {
  for (const [path, search] of [['/embed', ''], ['/embed/session', ''], ['/', '?mode=compact'], ['/', '?entry=embed'], ['/', '?pet=1'], ['/', '?channel=1']]) {
    assert.deepEqual(resolveLauncherFace(search, true, true, path), { launcher: false, remember: true });
    const explicit = new URLSearchParams(search);
    explicit.set('launcher', '1');
    assert.deepEqual(resolveLauncherFace(explicit.toString(), true, false, path), { launcher: true, remember: true });
  }
});

test('storage failure still honors an explicit launcher entry', () => {
  const win = { top: {}, location: { search: '?launcher=1' }, get sessionStorage() { throw new Error('blocked'); } };
  win.top = win;
  assert.equal(isLauncherWindow(win as unknown as Window), true);
});
