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

/** A saved call's status. A saved call is over: one with no word never ended. */
const savedStatus = (t: SavedTool): ToolStatus => {
  const s = toolStatus(t.status);
  return s && s !== 'running' ? s : 'failed';
};

/** A history row's calls as the tool blocks of its bubble; `key` keeps
 *  their ids apart from other rows'. */
export function savedToolBlocks(tools: readonly SavedTool[] | undefined, key: string | number): ContentBlock[] {
  return (tools || []).map((t, i) => ({
    type: 'tool_use',
    id: `saved-${key}-${i}`,
    tool: t.tool,
    args: t.args || '',
    status: savedStatus(t),
  }));
}
