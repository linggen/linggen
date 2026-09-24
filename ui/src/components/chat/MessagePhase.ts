import type { ChatMessage } from '../../types';
import type { MessagePhase } from './types';

export const isTransientStatus = (entry: string): boolean => {
  const t = entry.trim();
  return t === 'Thinking...' || t === 'Thinking' || /^Thinking\s*\(/.test(t) ||
    t === 'Model loading...' || t === 'Model loading' || /^Model loading\s*\(/.test(t) ||
    t === 'Running' || /^Running skill:/.test(t);
};

/** Tool status line prefixes — these are already rendered by the tool widget, so text blocks
 *  containing only a status line should be suppressed to avoid duplication. */
export const TOOL_STATUS_PREFIXES = [
  'Reading file: ', 'Read file: ', 'Read failed: ',
  'Writing file: ', 'Wrote file: ', 'Write failed: ',
  'Editing file: ', 'Edited file: ', 'Edit failed: ',
  'Running command: ', 'Ran command: ', 'Command failed: ',
  'Searching: ', 'Searched: ', 'Search failed: ',
  'Listing files: ', 'Listed files: ', 'List files failed: ',
  'Delegating to subagent: ', 'Delegated to subagent: ', 'Delegation failed: ',
  'Fetching URL: ', 'Fetched URL: ', 'Fetch failed: ',
  'Searching web: ', 'Searched web: ', 'Web search failed: ',
  'Calling tool: ', 'Used tool: ', 'Tool failed: ',
];

/** Returns true if the text is purely a tool status line that duplicates the tool widget. */
export const isToolStatusText = (text: string): boolean => {
  const trimmed = text.trim();
  if (!trimmed) return false;
  return TOOL_STATUS_PREFIXES.some(p => trimmed.startsWith(p));
};

export function getMessagePhase(msg: ChatMessage): MessagePhase {
  if (!msg.isGenerating) return 'done';
  const hasRunningBlock = (msg.content || []).some(b => b.type === 'tool_use' && b.status === 'running');
  if (hasRunningBlock) return 'working';
  const entries = msg.activityEntries || [];
  const nonTransient = entries.filter(e => !isTransientStatus(e));
  if (nonTransient.length > 0) return 'working';
  if (msg.isThinking) return 'thinking';
  return 'streaming';
}
