import assert from 'node:assert/strict';
import { test } from 'node:test';
import { watchDialogEscape } from '../src/lib/dialogEscape.mts';

function escape(eventOptions: { repeat?: boolean; isComposing?: boolean } = {}) {
  return Object.assign(new Event('keydown', { cancelable: true }), { key: 'Escape', ...eventOptions });
}

// An isolated capture/bubble harness so a descendant can consume Escape
// between the two phases just as it can in the DOM.
function keyboard() {
  const capture = new EventTarget();
  const bubble = new EventTarget();
  const target = {
    addEventListener: (type: string, listener: EventListener, before?: boolean) => (before ? capture : bubble).addEventListener(type, listener),
    removeEventListener: (type: string, listener: EventListener, before?: boolean) => (before ? capture : bubble).removeEventListener(type, listener),
  };
  return {
    target,
    press(event = escape(), descendant?: (event: Event) => void) {
      capture.dispatchEvent(event);
      descendant?.(event);
      bubble.dispatchEvent(event);
    },
  };
}

test('Escape gives a descendant first refusal; the next press can close its dialog', () => {
  const keys = keyboard();
  let closed = 0;
  const stop = watchDialogEscape(keys.target, () => true, () => { closed++; });
  keys.press(escape(), (event) => event.preventDefault());
  keys.press(escape({ repeat: true }));
  keys.press(escape({ isComposing: true }));
  assert.equal(closed, 0);
  keys.press();
  assert.equal(closed, 1);
  stop();
  keys.press();
  assert.equal(closed, 1);
});

test('only the dialog that was topmost before Escape can close, regardless of listener order', () => {
  for (const order of [['outer', 'inner'], ['inner', 'outer']]) {
    const keys = keyboard();
    let top = 'inner';
    const closed: string[] = [];
    for (const dialog of order) {
      watchDialogEscape(keys.target, () => top === dialog, () => {
        closed.push(dialog);
        top = dialog === 'inner' ? 'outer' : '';
      });
    }
    keys.press();
    assert.deepEqual(closed, ['inner']);
    keys.press();
    assert.deepEqual(closed, ['inner', 'outer']);
  }
});
