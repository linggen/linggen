import { useCallback, useMemo, useState } from 'react';

/** A set of names held in state: which rows are expanded, which are busy. */
export interface NameSet {
  has(name: string): boolean;
  toggle(name: string): void;
  /** Mark `name` while `fn` runs; cleared when it settles either way. */
  busy<T>(name: string, fn: () => Promise<T>): Promise<T>;
}

export function useNameSet(): NameSet {
  const [names, setNames] = useState<ReadonlySet<string>>(() => new Set());

  const set = useCallback((name: string, on?: boolean) => {
    setNames((prev) => {
      const next = new Set(prev);
      if (on ?? !next.has(name)) next.add(name);
      else next.delete(name);
      return next;
    });
  }, []);

  return useMemo<NameSet>(() => ({
    has: (name) => names.has(name),
    toggle: (name) => set(name),
    busy: async (name, fn) => {
      set(name, true);
      try {
        return await fn();
      } finally {
        set(name, false);
      }
    },
  }), [names, set]);
}
