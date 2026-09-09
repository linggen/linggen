/**
 * Derived run information — running main run IDs by agent.
 */
import { useMemo } from 'react';
import { useServerStore } from '../stores/serverStore';

export function useRunInfo() {
  const agentRuns = useServerStore((s) => s.agentRuns);

  const sortedAgentRuns = useMemo(() => {
    const statusScore = (status: string) => (status === 'running' ? 1 : 0);
    return [...agentRuns].sort(
      (a, b) => statusScore(b.status) - statusScore(a.status) || Number(b.started_at || 0) - Number(a.started_at || 0),
    );
  }, [agentRuns]);

  const runningMainRunIds = useMemo(() => {
    const out: Record<string, string> = {};
    for (const run of sortedAgentRuns) {
      const agentId = run.agent_id.toLowerCase();
      if (run.parent_run_id || run.status !== 'running') continue;
      if (!out[agentId]) out[agentId] = run.run_id;
    }
    return out;
  }, [sortedAgentRuns]);

  /** The top-level run in flight for each session.
   *
   *  "Stop" means the turn running in front of you, so the composer's stop
   *  button keys on the session rather than on whichever agent happens to be
   *  selected — an embedded skill chat has no reason to have selected the
   *  agent that owns its run, and 2026-09-09 it had not: Pulse's chat ran on
   *  `ling` and showed no stop button at all. */
  const runningRunIdBySession = useMemo(() => {
    const out: Record<string, string> = {};
    for (const run of sortedAgentRuns) {
      if (run.parent_run_id || run.status !== 'running' || !run.session_id) continue;
      if (!out[run.session_id]) out[run.session_id] = run.run_id;
    }
    return out;
  }, [sortedAgentRuns]);

  return { runningMainRunIds, runningRunIdBySession };
}
