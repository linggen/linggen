/**
 * A run's live bubble and its saved rows, bound by the run's id — never by
 * agent, words or time (chat-spec.md § Runs). Every event a run sends and
 * every row it saves carries `run_id`; here the chat:
 *  - finds the bubble an event of a run lands in (`runBubbleIndex`), so a
 *    stopped run's late events never reach the next turn's bubble;
 *  - decides, for display, whether a run shows as its bubble or as its saved
 *    rows (`bindRuns`).
 * Rows without a run id (saved before 2026-10-22) never come here: they keep
 * the old matching in messageUtils.mergeChatMessages. Pure, so node tests
 * can hold it.
 */
import type { ChatMessage } from '../types.ts';

/** A bubble this page built from a run's events (not a saved row). */
export const isRunBubble = (m: ChatMessage, runId?: string): boolean =>
  !!m.live && !!m.runId && (runId == null || m.runId === runId);

/**
 * A run bubble's work up to a message the run took in mid-turn (`fold.rs`):
 * a finished block of its own, bound to no live run, so it stays above the
 * message while the run's later events open a new bubble below it. The next
 * load's saved rows replace it (`continued` row, chat-spec § Message queue).
 */
export function sealedSegment(bubble: ChatMessage): ChatMessage {
  return {
    ...bubble,
    runId: `${bubble.runId}~before-message`,
    live: false,
    isGenerating: false,
    isThinking: false,
    text: bubble.text || bubble.liveText || '',
    liveText: undefined,
  };
}

/** A saved row of a run (from the history view). */
export const isRunRow = (m: ChatMessage): boolean => !!m.runId && !m.live;

/**
 * Where an event of `agentId`'s run `runId` lands: that run's bubble; else a
 * placeholder this turn opened before its run began (the pre-run "Loading
 * model…" carries no run) — it becomes the run's. -1: a new bubble.
 * Without a run id (an event no run sent): the agent's latest generating
 * bubble that belongs to no run.
 */
export function runBubbleIndex(msgs: readonly ChatMessage[], agentId: string, runId?: string): number {
  if (runId) {
    const own = msgs.findIndex((m) => isRunBubble(m, runId));
    if (own >= 0) return own;
  }
  for (let i = msgs.length - 1; i >= 0; i -= 1) {
    const m = msgs[i];
    if (m.from === agentId && m.isGenerating && !m.runId) return i;
  }
  return -1;
}

/** What makes a bubble a run's: its id, and how many of the run's saved rows
 *  were already on screen when it took the run — those it does not hold. */
export function bubbleOfRun(msgs: readonly ChatMessage[], runId?: string): Partial<ChatMessage> {
  if (!runId) return {};
  const seenSaved = msgs.filter((m) => isRunRow(m) && m.runId === runId).length;
  return { runId, live: true, seenSaved };
}

/** `msgs` with bubble `idx` made run `runId`'s (a placeholder adopted). */
export function claimForRun(msgs: ChatMessage[], idx: number, runId?: string): ChatMessage[] {
  if (!runId || idx < 0 || msgs[idx].runId) return msgs;
  const next = [...msgs];
  next[idx] = { ...next[idx], ...bubbleOfRun(msgs, runId) };
  return next;
}

const textBlocks = (m: ChatMessage) =>
  (m.content || []).filter((b) => b.type === 'text' && (b.text || '').trim()).length;
const callBlocks = (m: ChatMessage) => (m.content || []).filter((b) => b.type === 'tool_use').length;
const spoken = (rows: readonly ChatMessage[]) =>
  rows.filter((r) => !r.interrupted && r.text.trim()).length;
const calls = (rows: readonly ChatMessage[]) =>
  rows.reduce((n, r) => n + (r.content || []).filter((b) => b.type === 'tool_use').length, 0);

/** The bubble holds the whole run: every call and every piece of words its
 *  saved rows have. One that missed events (a reconnect, a long switch
 *  away) holds less, and loses to the rows. */
export function holdsRun(bubble: ChatMessage, rows: readonly ChatMessage[]): boolean {
  return callBlocks(bubble) >= calls(rows) && textBlocks(bubble) >= spoken(rows);
}

/** The finished bubble with what only its rows know: that the run stopped
 *  (its calls left running are failed — they never came back), and its
 *  words when the bubble lost them. */
export function absorbRows(bubble: ChatMessage, rows: readonly ChatMessage[]): ChatMessage {
  const cut = rows.find((r) => r.interrupted);
  const last = [...rows].reverse().find((r) => !r.interrupted && r.text.trim());
  let out = bubble;
  if (cut) {
    out = {
      ...out,
      interrupted: true,
      runEndedMs: cut.runEndedMs,
      content: (out.content || []).map((b) => (b.status === 'running' ? { ...b, status: 'failed' as const } : b)),
    };
  }
  if (last && !out.text.trim()) out = { ...out, text: last.text, isError: last.isError || out.isError };
  return out;
}

/** One run as the chat shows it. */
function showRun(bubble: ChatMessage, rows: readonly ChatMessage[]): ChatMessage[] {
  if (rows.length === 0) return [bubble];
  if (bubble.isGenerating) return [...rows.slice(0, bubble.seenSaved ?? 0), bubble];
  if (holdsRun(bubble, rows)) return [absorbRows(bubble, rows)];
  return [...rows];
}

/**
 * The chat as shown: each run with a bubble here shows as that bubble —
 * while it runs, after the saved rows it did not see; once it is done, the
 * bubble when it holds the whole run, else the saved rows. A run with no
 * bubble shows its rows. Everything without a run id passes through.
 */
export function bindRuns(msgs: readonly ChatMessage[]): ChatMessage[] {
  const rowsOf = new Map<string, ChatMessage[]>();
  for (const m of msgs) {
    if (!isRunRow(m)) continue;
    rowsOf.set(m.runId!, [...(rowsOf.get(m.runId!) || []), m]);
  }
  const bubbled = new Set(msgs.filter((m) => isRunBubble(m)).map((m) => m.runId!));
  const out: ChatMessage[] = [];
  for (const m of msgs) {
    if (isRunBubble(m)) out.push(...showRun(m, rowsOf.get(m.runId!) || []));
    else if (!isRunRow(m) || !bubbled.has(m.runId!)) out.push(m);
  }
  return out;
}
