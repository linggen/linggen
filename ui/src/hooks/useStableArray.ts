import { useState } from 'react';

/** Returns the previous array while `next` holds the same elements (by
 *  identity), so a memoized child keeps its props equal across renders.
 *  Uses React's "adjust state during render" pattern: no ref reads. */
export function useStableArray<T>(next: T[]): T[] {
  const [stable, setStable] = useState(next);
  const same = stable.length === next.length && next.every((x, i) => x === stable[i]);
  if (!same) {
    setStable(next);
    return next;
  }
  return stable;
}
