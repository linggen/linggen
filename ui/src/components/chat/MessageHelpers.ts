import type { ChatMessage } from '../../types';
import { sanitizeAgentMessageText } from '../../lib/messageUtils';

export const statusBadgeClass = (status?: string) => {
  if (status === 'working') return 'bg-green-500/15 text-green-600 dark:text-green-300';
  if (status === 'thinking') return 'bg-blue-500/15 text-blue-600 dark:text-blue-300';
  if (status === 'calling_tool') return 'bg-amber-500/15 text-amber-700 dark:text-amber-300';
  if (status === 'model_loading') return 'bg-indigo-500/15 text-indigo-700 dark:text-indigo-300';
  return 'bg-slate-500/15 text-slate-600 dark:text-slate-300';
};

export const visibleMessageText = (msg: ChatMessage) => {
  if (msg.role === 'user') return msg.text;
  const sanitized = sanitizeAgentMessageText(msg.text);
  return sanitized.split('\n')
    .filter(line => {
      const t = line.trimStart();
      return !t.startsWith('Used tool:') &&
             !t.startsWith('Tool done:') &&
             !t.startsWith('Tool tool_not_allowed:') &&
             !t.startsWith('Tool error:') &&
             !t.startsWith('tool_not_allowed:') &&
             !t.startsWith('Delegated task:');
    })
    .join('\n').replace(/\n{3,}/g, '\n\n').trim();
};
