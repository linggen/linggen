/**
 * A press held still for a while — the second way to open an item's menu
 * (the first is its ⋯ button). Pure: the timer is passed in, so a test can
 * hold the clock.
 */

export const HOLD_MS = 500;
/** How far a finger may drift (px) before it is a scroll, not a hold. */
export const DRIFT_PX = 8;

export interface Clock {
  set: (run: () => void, ms: number) => unknown;
  clear: (handle: unknown) => void;
}

const real: Clock = { set: (run, ms) => setTimeout(run, ms), clear: (h) => clearTimeout(h as ReturnType<typeof setTimeout>) };

export function longPress(onHold: () => void, clock: Clock = real) {
  let handle: unknown = null;
  let from: { x: number; y: number } | null = null;
  const stop = () => {
    if (handle !== null) clock.clear(handle);
    handle = null;
    from = null;
  };
  return {
    down(x: number, y: number) {
      stop();
      from = { x, y };
      handle = clock.set(() => {
        handle = null;
        from = null;
        onHold();
      }, HOLD_MS);
    },
    move(x: number, y: number) {
      if (from && Math.hypot(x - from.x, y - from.y) > DRIFT_PX) stop();
    },
    up: stop,
    cancel: stop,
  };
}
