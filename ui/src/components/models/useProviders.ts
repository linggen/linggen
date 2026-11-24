/**
 * The one holder of what `GET /api/providers` answered, and the one way every
 * button of Settings → Models changes it: `run` calls a route, replaces the
 * list with the answer the route returned, then lets Settings read the config
 * again (`onChanged`). A refused call shows the route's fixed answer as a
 * short fixed label (lib/providers.mts `failureLabel`) and reads the list
 * again, so the page never stays on a copy the engine has left.
 */
import { useCallback, useEffect, useRef, useState } from 'react';
import { ApiError } from '../../lib/api';
import { providersApi } from '../../lib/endpoints';
import { failureLabel, type ProvidersAnswer } from '../../lib/providers.mts';
import type { Word } from '../../lib/modelsWords.mts';

export type Run = (call: () => Promise<ProvidersAnswer | void>) => Promise<boolean>;

export function useProviders(onChanged: () => Promise<void>) {
  const [answer, setAnswer] = useState<ProvidersAnswer | null>(null);
  const [loadFailed, setLoadFailed] = useState(false);
  const [problem, setProblem] = useState<Word | null>(null);
  const [busy, setBusy] = useState(false);
  const changed = useRef(onChanged);
  changed.current = onChanged;

  const load = useCallback(async () => {
    try {
      setAnswer(await providersApi.list());
      setLoadFailed(false);
    } catch {
      setLoadFailed(true);
    }
  }, []);
  useEffect(() => { void load(); }, [load]);

  const checking = answer?.providers.some((p) => p.families?.families.some((f) => f.hold === 'checking'));
  const tries = useRef(0);
  useEffect(() => {
    if (!checking) { tries.current = 0; return; }
    if (busy || loadFailed || tries.current >= 16) return;
    const timer = setTimeout(() => { tries.current++; void load(); }, 750);
    return () => clearTimeout(timer);
  }, [answer, busy, checking, load, loadFailed]);

  const run: Run = useCallback(async (call) => {
    setBusy(true);
    setProblem(null);
    try {
      const next = await call();
      if (next) setAnswer(next);
      await changed.current();
      return true;
    } catch (e) {
      const refused = e instanceof ApiError;
      setProblem(failureLabel(refused ? e.status : undefined, refused ? e.message : ''));
      void load();
      return false;
    } finally {
      setBusy(false);
    }
  }, [load]);

  return { answer, loadFailed, problem, busy, load, run };
}
