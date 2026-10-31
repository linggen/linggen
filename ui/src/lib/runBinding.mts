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

const SEALED = '~before-message';

/**
 * A run bubble's work up to a message the run took in mid-turn (`fold.rs`):
 * a finished block of its own, bound to no live run, so it stays above the
 * message while the run's later events open a new bubble below it. A load
 * binds it to the run's saved rows before that message (`bindSealed`).
 */
export function sealedSegment(bubble: ChatMessage): ChatMessage {
  return {
    ...bubble,
    runId: `${bubble.runId}${SEALED}`,
    live: false,
    isGenerating: false,
    isThinking: false,
    text: bubble.text || bubble.liveText || '',
    liveText: undefined,
  };
}

/** A saved row of a run (from the history view), or a sealed block. */
export const isRunRow = (m: ChatMessage): boolean => !!m.runId && !m.live;

/** The run a sealed block was cut from; undefined for anything else. */
const sealedRunOf = (m: ChatMessage): string | undefined =>
  !m.live && m.runId?.endsWith(SEALED) ? m.runId.slice(0, -SEALED.length) : undefined;

export const isSealed = (m: ChatMessage): boolean => sealedRunOf(m) != null;

const isPersonRow = (m: ChatMessage) => m.role === 'user' || m.from === 'user';

/** Run `runId`'s saved rows, split at each message the person sent while it
 *  ran: part k is the run's work before the (k+1)-th such message. */
function rowsByMessage(persisted: readonly ChatMessage[], runId: string): ChatMessage[][] {
  const parts: ChatMessage[][] = [[]];
  for (const m of persisted) {
    const last = parts[parts.length - 1];
    if (isRunRow(m) && m.runId === runId) last.push(m);
    else if (isPersonRow(m) && last.length > 0) parts.push([]);
  }
  return parts;
}

/**
 * A load's saved rows against this page's sealed blocks. The k-th sealed
 * block of a run is that run's rows before the k-th message it took in: it
 * shows as the block when the block holds them, else as those rows. Either
 * way it is the run going on — never a run cut off — and it stays above the
 * message, apart from the run's live bubble below it.
 */
export function bindSealed(
  persisted: readonly ChatMessage[],
  sealed: readonly ChatMessage[],
): { rows: ChatMessage[]; sealed: ChatMessage[] } {
  const dropped = new Set<ChatMessage>();
  const retagged = new Map<ChatMessage, ChatMessage>();
  const kept: ChatMessage[] = [];
  const runs = new Set(sealed.map(sealedRunOf));
  for (const run of runs) {
    const blocks = sealed.filter((m) => sealedRunOf(m) === run);
    const parts = rowsByMessage(persisted, run!);
    blocks.forEach((block, k) => {
      const rows = parts[k] || [];
      // The last part is the work after the newest message: the live bubble's.
      if (rows.length === 0 || k >= parts.length - 1) {
        kept.push(block);
        return;
      }
      if (holdsRun(block, rows)) {
        kept.push(block);
        rows.forEach((r) => dropped.add(r));
        return;
      }
      for (const r of rows) {
        retagged.set(r, { ...r, runId: block.runId, interrupted: undefined, runEndedMs: undefined });
      }
    });
  }
  const rows = persisted.filter((m) => !dropped.has(m)).map((m) => retagged.get(m) ?? m);
  return { rows, sealed: kept };
}

/** A live bubble of a run that took a message in sees only rows after it;
 *  the rows before are its sealed block's. */
export function afterSealed(bubble: ChatMessage, sealed: readonly ChatMessage[]): ChatMessage {
  const cut = sealed.some((m) => sealedRunOf(m) === bubble.runId);
  return cut && bubble.seenSaved ? { ...bubble, seenSaved: 0 } : bubble;
}

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
