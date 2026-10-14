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
