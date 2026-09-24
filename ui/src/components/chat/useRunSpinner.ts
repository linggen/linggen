// The run line under the chat: whether the session is busy, a verb, the
// elapsed time, and the "{verb} for Xs" summary once it stops. Kept per
// session, so switching between two busy sessions shows each one's own time.
import { useEffect, useRef, useState } from 'react';
import { useServerStore, isSessionBusy } from '../../stores/serverStore';

const VERBS = [
  'Thinking', 'Pondering', 'Brewing', 'Cogitating', 'Reticulating',
  'Noodling', 'Musing', 'Simmering', 'Percolating', 'Ruminating',
  'Contemplating', 'Marinating', 'Conjuring', 'Scheming', 'Tinkering',
  'Crafting', 'Hatching', 'Computing', 'Deliberating',
];

export interface RunSummary { verb: string; elapsed: number; interrupted?: boolean }

export function useRunSpinner(sessionId: string | null | undefined) {
  const key = sessionId || '';
  // One definition of busy for every surface (serverStore.isSessionBusy).
  const isAgentActive = useServerStore((s) => isSessionBusy(s, sessionId));
  const runsRef = useRef(new Map<string, { start: number; verb: string }>());
  const [spinnerVerb, setSpinnerVerb] = useState('');
  const [thinkingElapsed, setThinkingElapsed] = useState(0);
  const [summaries, setSummaries] = useState<Record<string, RunSummary | null>>({});

  useEffect(() => {
    const runs = runsRef.current;
    if (isAgentActive) {
      let run = runs.get(key);
      if (!run) {
        run = { start: Date.now(), verb: VERBS[Math.floor(Math.random() * VERBS.length)] };
        runs.set(key, run);
        setSummaries((s) => ({ ...s, [key]: null }));
      }
      const { start } = run;
      setSpinnerVerb(run.verb);
      const tick = () => setThinkingElapsed(Math.floor((Date.now() - start) / 1000));
      tick();
      const interval = setInterval(tick, 500);
      return () => clearInterval(interval);
    }
    const run = runs.get(key);
    if (run) {
      runs.delete(key);
      const elapsed = Math.floor((Date.now() - run.start) / 1000);
      // A send that failed after the spinner started never ran a turn —
      // nothing happened, so there's nothing to summarize.
      const failedAt = useServerStore.getState().sendFailedAt[key] || 0;
      if (elapsed > 0 && failedAt < run.start) {
        setSummaries((s) => ({ ...s, [key]: { verb: run.verb || 'Worked', elapsed } }));
      }
    }
    setThinkingElapsed(0);
  }, [isAgentActive, key]);

  // A run that died server-side without a terminal event (daemon killed or
  // restarted mid-turn) is found by page_state reconciliation after the
  // summary is already up; "Musing for 3m 5s" would read as still working.
  const runInterrupted = useServerStore((s) => s.runInterruptedAt[key]);
  useEffect(() => {
    if (!runInterrupted || !key) return;
    useServerStore.getState().consumeRunInterrupted(key);
    setSummaries((s) => (s[key] ? { ...s, [key]: { ...s[key]!, interrupted: true } } : s));
  }, [runInterrupted, key]);

  return { isAgentActive, spinnerVerb, thinkingElapsed, lastRunSummary: summaries[key] ?? null };
}
