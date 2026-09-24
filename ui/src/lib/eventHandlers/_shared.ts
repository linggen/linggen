/**
 * Helpers shared across per-kind event handlers.
 */
import type { UiEvent } from '../../types';
import { useSessionStore } from '../../stores/sessionStore';

// ---------------------------------------------------------------------------
// Permission suppress — prevents page_state from overwriting optimistic mode changes
// ---------------------------------------------------------------------------

let _permissionSuppressedUntil = 0;

/** Call after a user-initiated permission mode change to suppress page_state
 *  overwrites for a short window (3s, enough for the PATCH to propagate). */
export function suppressPermissionSync(): void {
  _permissionSuppressedUntil = Date.now() + 3000;
}

/** True while the suppression window is active. */
export function isPermissionSuppressed(): boolean {
  return Date.now() < _permissionSuppressedUntil;
}

// ---------------------------------------------------------------------------
// Session-id resolution
// ---------------------------------------------------------------------------

/** Resolve a session ID for keying status maps. Prefer the event's session_id,
 *  fall back to the currently active session. */
export function getSessionId(item: UiEvent): string {
  return item.session_id || useSessionStore.getState().activeSessionId || '';
}

// ---------------------------------------------------------------------------
// Tool activity text parser
// ---------------------------------------------------------------------------

export const toolPrefixMap: [string, string][] = [
  ['Reading file: ', 'Read'],
  ['Read file: ', 'Read'],
  ['Read failed: ', 'Read'],
  ['Writing file: ', 'Write'],
  ['Wrote file: ', 'Write'],
  ['Write failed: ', 'Write'],
  ['Editing file: ', 'Edit'],
  ['Edited file: ', 'Edit'],
  ['Edit failed: ', 'Edit'],
  ['Running command: ', 'Bash'],
  ['Ran command: ', 'Bash'],
  ['Command failed: ', 'Bash'],
  ['Searching: ', 'Grep'],
  ['Searched: ', 'Grep'],
  ['Search failed: ', 'Grep'],
  ['Listing files: ', 'Glob'],
  ['Listed files: ', 'Glob'],
  ['List files failed: ', 'Glob'],
  ['Delegating to subagent: ', 'Task'],
  ['Delegated to subagent: ', 'Task'],
  ['Delegation failed: ', 'Task'],
  ['Fetching URL: ', 'WebFetch'],
  ['Fetched URL: ', 'WebFetch'],
  ['Fetch failed: ', 'WebFetch'],
  ['Searching web: ', 'WebSearch'],
  ['Searched web: ', 'WebSearch'],
  ['Web search failed: ', 'WebSearch'],
  ['Calling tool: ', 'Tool'],
  ['Used tool: ', 'Tool'],
  ['Tool failed: ', 'Tool'],
];

/** Build a user-facing status line for a tool start event. */
export function formatToolStartLine(toolName: string, argsStr: string): string {
  try {
    const args = JSON.parse(argsStr);
    switch (toolName) {
      case 'Read': return `Reading file: ${args.file_path || args.path || argsStr}`;
      case 'Write': return `Writing file: ${args.file_path || args.path || argsStr}`;
      case 'Edit': return `Editing file: ${args.file_path || args.path || argsStr}`;
      case 'Bash': {
        const cmd = args.command || args.cmd || '';
        return `Running command: ${cmd.length > 80 ? cmd.slice(0, 77) + '...' : cmd}`;
      }
      case 'Grep': return `Searching: ${args.pattern || argsStr}`;
      case 'Glob': return `Listing files: ${args.pattern || argsStr}`;
      case 'Task':
      case 'delegate_to_agent':
        return `Delegating to subagent: ${args.agent_id || args.agent || argsStr}`;
      case 'WebFetch': return `Fetching URL: ${args.url || argsStr}`;
      case 'WebSearch': return `Searching web: ${args.query || argsStr}`;
      default: return `Calling tool: ${toolName}`;
    }
  } catch {
    return `Calling tool: ${toolName}`;
  }
}
