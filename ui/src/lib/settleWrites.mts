/**
 * A write that waits for the person to stop changing things for a moment —
 * and is never lost when they leave first. `later(run)` (re)starts the quiet
 * time with the newest write; `cancel()` drops it (nothing left to write);
 * `flush()` writes it now (the page is going). Pure, so node tests can hold
 * it.
 */
export function settleWrites(quietMs: number) {
  let timer: ReturnType<typeof setTimeout> | null = null;
  let pending: (() => void) | null = null;

  const cancel = () => {
    if (timer) clearTimeout(timer);
    timer = null;
    pending = null;
  };
  const flush = () => {
    const run = pending;
    cancel();
    run?.();
  };
  const later = (run: () => void) => {
    cancel();
    pending = run;
    timer = setTimeout(flush, quietMs);
  };
  return { later, cancel, flush };
}
