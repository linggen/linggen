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
