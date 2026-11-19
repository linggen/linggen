import type { ChatMessage } from '../types.ts';

/** The main chat holds every member's traffic and unrouted notices: what the
 *  page itself says (`system`, e.g. a `!` command's output) and the lines the
 *  person typed to it (a `!` command), whoever is selected. */
function isMainChatRow(msg: ChatMessage, selected: string, mainAgentIds: readonly string[]): boolean {
  const from = (msg.from || msg.role).trim().toLowerCase();
  const to = (msg.to || '').trim().toLowerCase();
  if (from === 'system' || to === 'system' || from === 'compaction' || from === 'assistant') return true;
  if (from === selected || to === selected) return true;
  if (mainAgentIds.includes(from) || mainAgentIds.includes(to)) return true;
  return (msg.role === 'user' || from === 'user') && !to;
}

/** A place is told only in a daily session. Other places' rows are kept
 *  for the fold even when this page does not know their speakers. */
export function selectMainChatRows(messages: ChatMessage[], opts: {
  selected: string;
  mainAgentIds?: readonly string[];
  here: string;
  runPlaces?: Record<string, string>;
}): ChatMessage[] {
  const { selected, mainAgentIds = [], here, runPlaces = {} } = opts;
  return messages.filter((m) => {
    const place = m.place || (m.runId ? runPlaces[m.runId] : undefined);
    return (!!place && place !== here) || isMainChatRow(m, selected, mainAgentIds);
  });
}
