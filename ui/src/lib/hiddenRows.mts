import type { ChatMessage } from '../types.ts';

// Rows the chat never shows as anyone's words. A prompt the engine wrote to
// an agent carries `[HIDDEN]` (the engine sets it on every kickoff it
// authors — heralds, ambient moments, a page's report); the model reads it,
// the chat does not. Rows saved before that marker are left out by the
// history endpoints (`resident::is_engine_prompt`), not matched here.
export const shouldHideInternalChatMessage = (_from?: string, text?: string): boolean => {
  if (!text) return false;
  // Hide [HIDDEN] messages (system prompts from app skills)
  if (text.startsWith('[HIDDEN]')) return true;
  if (text.startsWith('Starting autonomous loop for task:')) return true;
  // Hide raw tool observation messages that leak from context (e.g. "Tool Bash: Bash output...")
  if (/^Tool\s+\w+:/i.test(text)) return true;
  // Hide "Used tool: Read · target=..." lines rendered from sanitization
  if (text.startsWith('Used tool:')) return true;
  // Hide delegation task text (already shown in SubagentTreeView)
  if (text.startsWith('Delegated task:')) return true;
  return false;
};

/** Display policy before binding and folding, so a silent run cannot leave a tool row.
 *  The world's context block and a leaked tool observation are never shown;
 *  a note the page itself wrote (`system`, e.g. a `!` command's output) is
 *  the person's own and always is. */
export function visibleChatRows(messages: ChatMessage[]): ChatMessage[] {
  const isYinyueSilence = (m: ChatMessage) => m.from === 'yinyue' && m.silent;
  const silentRuns = new Set(messages.filter(isYinyueSilence).map((m) => m.runId).filter(Boolean));
  return messages.filter((m) => {
    if (m.from === 'world' || shouldHideInternalChatMessage(m.from, m.text)) return false;
    if (isYinyueSilence(m) || (m.from === 'yinyue' && m.runId && silentRuns.has(m.runId))) return false;
    return !m.silent || !!m.content?.some((b) => b.type === 'tool_use');
  });
}

// The spoken contract's word for "say nothing" — the engine's
// `resident::spoken::SILENT`, mirrored here (a Rust test locks the two).
// The engine marks a silent reply (`silent` on its message event, and leaves
// its word out of history); this word only keeps it from streaming in first.
export const SILENT = 'SILENT';

/** Text that, streaming, may still turn out to be the contract's silence —
 *  held back until it says otherwise. */
export const mayBecomeSilence = (text?: string): boolean => {
  const t = (text || '').trim().replace(/[.!…]+$/, '').toUpperCase();
  return t.length > 0 && SILENT.startsWith(t);
};
