/**
 * A run that ended without a reply, as the history brings it back: the
 * engine sends one row per such run (`run: "interrupted"`, its calls in
 * `tools`) — see src/server/chat/run_calls.rs. Here it becomes the
 * agent's row with the run's tool calls, marked interrupted. Pure, so node
 * tests can hold it.
 */
import type { ChatMessage } from '../types.ts';
import { savedToolBlocks, type SavedTool } from './toolStatus.mts';

/** The fields of a history row's header this reads. */
export interface RunMeta {
  from: string;
  to?: string;
  ts: number;
  ended?: number;
  run?: string;
  /** The run (absent on rows saved before 2026-10-22). */
  run_id?: string;
  /** The calls made before this row (a spoken row), or the run's (its own row). */
  tools?: SavedTool[];
  [key: string]: unknown;
}

/** Said under the tool calls of a run that stopped before it replied. */
export const INTERRUPTED_RUN_TEXT = 'Interrupted';

export const isInterruptedRunMeta = (meta: RunMeta): boolean => meta.run === 'interrupted';

/** The agent's row for an interrupted run: its calls, no words. Text stays
 *  empty — tool calls are work, not a reply (see interruptedTurn.mts). */
export function interruptedRunMessage(meta: RunMeta): ChatMessage {
  const startMs = Number(meta.ts || 0) * 1000;
  const content = savedToolBlocks(meta.tools, `run-${meta.ts}`);
  return {
    role: 'agent',
    from: meta.from,
    to: meta.to,
    text: '',
    timestamp: new Date(startMs).toLocaleTimeString(),
    timestampMs: startMs,
    content,
    interrupted: true,
    runEndedMs: Number(meta.ended || meta.ts || 0) * 1000,
    ...(meta.run_id ? { runId: meta.run_id } : {}),
  };
}

/** Same agent, compared the way chat rows are. */
const sameAgent = (a?: string, b?: string) =>
  (a || '').trim().toLowerCase() === (b || '').trim().toLowerCase();

/** Saved rows stamp whole seconds and the live block the page's clock: the
 *  run's first call and its bubble's first block differ by about this. */
const CALL_STAMP_SLACK_MS = 5_000;

const toolNames = (m: ChatMessage): string[] =>
  (m.content || []).filter((b) => b.type === 'tool_use').map((b) => b.tool || '');

/** This run's row as an earlier merge left it: claimed again, so a second
 *  load finds the run where it is and never goes looking for another bubble.
 *  Matched by the run's end — the merge may have moved its start to the
 *  bubble's. */
const isMergedRow = (run: ChatMessage, m: ChatMessage): boolean =>
  !!m.interrupted && sameAgent(m.from, run.from) && m.runEndedMs === run.runEndedMs;

/** A bubble that streamed this run: the same calls, in order, the first one
 *  made when the run's first call was. A later turn's bubble — the next
 *  message's, even one asking for the same tool — began after the run ended. */
const streamedRun = (run: ChatMessage, m: ChatMessage): boolean => {
  if (m.interrupted || m.role !== 'agent' || !sameAgent(m.from, run.from)) return false;
  const names = toolNames(m);
  if (names.length === 0 || names.join('\n') !== toolNames(run).join('\n')) return false;
  const firstCall = (m.content || []).find((b) => b.type === 'tool_use')?.timestampMs;
  if (firstCall == null) return false;
  const start = (run.timestampMs ?? 0) - CALL_STAMP_SLACK_MS;
  const end = (run.runEndedMs ?? run.timestampMs ?? 0) + CALL_STAMP_SLACK_MS;
  return firstCall >= start && firstCall <= end;
};

/**
 * LEGACY — only for an interrupted row with no `run_id` (saved before
 * 2026-10-22); a row with one binds by it (runBinding.mts).
 * The live row that shows this run, if this client holds one: the run's row
 * from an earlier merge, else the bubble that streamed it. That bubble IS
 * the run — the saved row only adds that it stopped. Never another turn's
 * bubble: a time window alone let a stopped run take the next turn's
 * calls and answer (2026-10-22).
 */
export function liveBubbleOfRun(run: ChatMessage, live: readonly ChatMessage[], taken: ReadonlySet<number>): number {
  const free = (i: number) => !taken.has(i);
  const merged = live.findIndex((m, i) => free(i) && isMergedRow(run, m));
  if (merged >= 0) return merged;
  return live.findIndex((m, i) => free(i) && streamedRun(run, m));
}

/** Show the small note on an interrupted run unless the next row already
 *  says the turn stopped (the "No response" line with its Resend). */
export function showsInterruptedNote(msg: ChatMessage, next?: ChatMessage): boolean {
  if (!msg.interrupted) return false;
  return !(next?.isError && next.resend);
}
