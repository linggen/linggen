/**
 * A native session's subagent rows against the engine's runs — the runs are
 * the truth. Live events keep the rows current, but an event lost while the
 * session's channel was closed, or a row saved while its child still ran,
 * would otherwise show "running" forever; and a page opened while children
 * run would show none. Each page_state's runs settle both.
 */
import type { AgentRunInfo, SubagentTreeEntry } from '../types';

/** A row this young may be ahead of the runs list it is checked against. */
export const SETTLE_GRACE_MS = 15_000;

/** The running rows whose run is no longer running end; the rest stay. */
export function settleTree(
  tree: SubagentTreeEntry[],
  live: Set<string>,
  now: number,
): SubagentTreeEntry[] {
  let changed = false;
  const next = tree.map((entry) => {
    const young = entry.timestampMs !== undefined && now - entry.timestampMs < SETTLE_GRACE_MS;
    if (entry.status !== 'running' || live.has(entry.subagentId) || young) return entry;
    changed = true;
    return { ...entry, status: 'done' as const, currentActivity: null };
  });
  return changed ? next : tree;
}

/** The running child runs no row shows yet. */
export function unshownChildren(runs: AgentRunInfo[], shown: Set<string>): AgentRunInfo[] {
  return runs.filter((run) => run.status === 'running' && run.parent_run_id && !shown.has(run.run_id));
}

/** A row for a child run the page never saw start. */
export function rowForRun(run: AgentRunInfo): SubagentTreeEntry {
  return {
    subagentId: run.run_id,
    agentName: run.agent_id,
    task: run.detail ?? '',
    status: 'running',
    toolCount: 0,
    contextTokens: 0,
    currentActivity: null,
    toolSteps: [],
    timestampMs: run.started_at * 1000,
  };
}
