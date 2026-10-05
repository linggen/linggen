/**
 * A run that ended without a reply, as the history brings it back: the
 * engine sends one row per such run (`run: "interrupted"`, its calls in
 * `tools`) — see src/server/chat/unanswered_run.rs. Here it becomes the
 * agent's row with the run's tool calls, marked interrupted. Pure, so node
 * tests can hold it.
 */
import type { ChatMessage, ContentBlock } from '../types.ts';

/** One call of the run, as the engine sends it (`args` is JSON text). */
export interface RunTool {
  tool: string;
  args?: string;
  done?: boolean;
}

/** The fields of a history row's header this reads. */
export interface RunMeta {
  from: string;
  to?: string;
  ts: number;
  ended?: number;
  run?: string;
  tools?: RunTool[];
  [key: string]: unknown;
}

/** Said under the tool calls of a run that stopped before it replied. */
export const INTERRUPTED_RUN_TEXT = 'Interrupted';

export const isInterruptedRunMeta = (meta: RunMeta): boolean => meta.run === 'interrupted';

/** The agent's row for an interrupted run: its calls, no words. Text stays
 *  empty — tool calls are work, not a reply (see interruptedTurn.mts). */
export function interruptedRunMessage(meta: RunMeta): ChatMessage {
  const startMs = Number(meta.ts || 0) * 1000;
  const content: ContentBlock[] = (meta.tools || []).map((t, i) => ({
    type: 'tool_use',
    id: `run-${meta.ts}-${i}`,
    tool: t.tool,
    args: t.args || '',
    status: t.done ? 'done' : 'failed',
  }));
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
  };
}

/** Same agent, compared the way chat rows are. */
const sameAgent = (a?: string, b?: string) =>
  (a || '').trim().toLowerCase() === (b || '').trim().toLowerCase();

/**
 * The live bubble that showed this run as it streamed, if this client still
 * holds it: the same agent's tool bubble from within the run's span. That
 * bubble IS the run — the saved row only adds that it stopped.
 */
export function liveBubbleOfRun(run: ChatMessage, live: readonly ChatMessage[], taken: ReadonlySet<number>): number {
  // A bubble opens as its turn starts, seconds before the first call.
  const start = (run.timestampMs ?? 0) - 30_000;
  const end = (run.runEndedMs ?? run.timestampMs ?? 0) + 120_000;
  return live.findIndex((m, i) =>
    !taken.has(i) &&
    !m.interrupted &&
    m.role === 'agent' &&
    sameAgent(m.from, run.from) &&
    (m.content || []).some((b) => b.type === 'tool_use') &&
    (m.timestampMs ?? 0) >= start &&
    (m.timestampMs ?? 0) <= end,
  );
}

/** Show the small note on an interrupted run unless the next row already
 *  says the turn stopped (the "No response" line with its Resend). */
export function showsInterruptedNote(msg: ChatMessage, next?: ChatMessage): boolean {
  if (!msg.interrupted) return false;
  return !(next?.isError && next.resend);
}
