import React from 'react';

/** A value read from the engine, read again on `reload` and when `deps` change. A late reply to an old ask is dropped. */
export function useLoad<T>(read: () => Promise<T>, deps: React.DependencyList) {
  const [value, setValue] = React.useState<T | null>(null);
  const [error, setError] = React.useState(false);
  const asked = React.useRef(0);
  const reload = React.useCallback(async () => {
    const mine = ++asked.current;
    try {
      const next = await read();
      if (mine !== asked.current) return;
      setValue(next);
      setError(false);
    } catch {
      if (mine === asked.current) setError(true);
    }
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, deps);
  React.useEffect(() => {
    setValue(null);
    reload();
  }, [reload]);
  return { value, error, reload };
}
