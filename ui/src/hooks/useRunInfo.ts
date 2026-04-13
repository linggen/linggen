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

  return { runningMainRunIds };
}
