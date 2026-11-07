/**
 * A tool call's status, one reading for both paths: live, from the engine's
 * block update (`status`), and saved, from the calls a history row carries
 * (`tools`, see src/server/chat/run_calls.rs). The engine words both the same
 * (`tool_call_status`), so a reload shows a call as it looked live. Pure, so
 * node tests can hold it.
 */
import type { ContentBlock } from '../types.ts';

export type ToolStatus = NonNullable<ContentBlock['status']>;

/** One call as a history row carries it (`args` is JSON text). */
export interface SavedTool {
  tool: string;
  args?: string;
  status?: string;
}

/** The engine's word for a call's status, or undefined for any other. */
export function toolStatus(status: unknown): ToolStatus | undefined {
  return status === 'running' || status === 'done' || status === 'failed' ? status : undefined;
}

/** A saved call's status. A call of a finished run is over: one with no
 *  word never ended. Only the live run's row (`live`) has calls running. */
const savedStatus = (t: SavedTool, live: boolean): ToolStatus => {
  const s = toolStatus(t.status);
  if (s === 'running') return live ? s : 'failed';
  return s ?? 'failed';
};

/** A history row's calls as the tool blocks of its bubble; `key` keeps
 *  their ids apart from other rows'. `live`: the row of the run still going
 *  (`run: "running"`), read by a page loaded mid-run. */
export function savedToolBlocks(
  tools: readonly SavedTool[] | undefined,
  key: string | number,
  live = false,
): ContentBlock[] {
  return (tools || []).map((t, i) => ({
    type: 'tool_use',
    id: `saved-${key}-${i}`,
    tool: t.tool,
    args: t.args || '',
    status: savedStatus(t, live),
  }));
}

/** A turn that ended has no call running: one whose update never came is
 *  over, failed — the rule a saved call follows (`savedStatus`), live. The
 *  same array when no call was left running. */
export function endedCalls(blocks: ContentBlock[] | undefined): ContentBlock[] | undefined {
  if (!blocks?.some((b) => b.type === 'tool_use' && b.status === 'running')) return blocks;
  return blocks.map((b) => (b.type === 'tool_use' && b.status === 'running' ? { ...b, status: 'failed' as const } : b));
}

const callsOf = (blocks: readonly ContentBlock[]) => blocks.filter((b) => b.type === 'tool_use');

/** A finished bubble's calls, each with the status the run's saved rows
 *  give it when the rows hold the same calls in the same order (an update
 *  the page missed cost it nothing); otherwise as `endedCalls` leaves them.
 *  Only a saved end (done / failed) counts: rows read mid-run (a switch back
 *  to the session) still say `running` for a call that ended since, and
 *  know less than the bubble. */
export function callsFromRows(
  blocks: ContentBlock[] | undefined,
  rowBlocks: readonly ContentBlock[],
): ContentBlock[] | undefined {
  const mine = callsOf(blocks || []);
  const saved = callsOf(rowBlocks);
  const same = mine.length === saved.length && mine.every((b, i) => b.tool === saved[i].tool);
  if (!same) return endedCalls(blocks);
  let i = 0;
  const out = (blocks || []).map((b) => {
    if (b.type !== 'tool_use') return b;
    const status = saved[i++].status;
    return status && status !== 'running' && status !== b.status ? { ...b, status } : b;
  });
  return endedCalls(out);
}
