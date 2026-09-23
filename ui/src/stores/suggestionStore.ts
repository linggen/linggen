/**
 * Suggestion buttons above the chat input, and the hint inside it.
 *
 * Three sources (doc/chat-spec.md § Suggestions): a skill's starters (read
 * from its frontmatter, not kept here), the page's buttons for what is on
 * screen, and the next-prompt hint forked after the last reply — the input's
 * grey text (Tab takes it); the row belongs to the page.
 */
import { create } from 'zustand';

/** The row never shows more than this. */
export const MAX_SUGGESTIONS = 4;

interface SuggestionState {
  /** Set by the app page around this chat — it outlives a New chat. Empty
   *  hands the row back to the starters. */
  page: string[];
  /** What each session's person would most likely type next; cleared when
   *  they send or a new turn starts. A page change leaves it: it lives in the
   *  input, the page in the row. */
  hints: Record<string, string>;
  setPage: (sessionId: string | null, items: unknown) => void;
  setHint: (sessionId: string, items: unknown) => void;
  clearHint: (sessionId: string) => void;
}

/** Button labels as the row shows them: trimmed, non-empty, distinct. */
export function cleanItems(items: unknown): string[] {
  if (!Array.isArray(items)) return [];
  const out: string[] = [];
  for (const item of items) {
    const text = typeof item === 'string' ? item.trim() : '';
    if (text && !out.includes(text)) out.push(text);
  }
  return out.slice(0, MAX_SUGGESTIONS);
}

/** What the row shows: the page's, else the hint as a button where the
 *  input can't show it ([hinted] false: a phone has no Tab), else the
 *  starters. */
export function visibleSuggestions(hint: string | undefined, page: string[], starters: string[] | undefined, hinted = true): string[] {
  if (page.length > 0) return page;
  if (hint) return hinted ? [] : [hint];
  return cleanItems(starters);
}

export const useSuggestionStore = create<SuggestionState>((set) => ({
  page: [],
  hints: {},
  setPage: (_sessionId, items) => set({ page: cleanItems(items) }),
  setHint: (sessionId, items) => {
    const [hint] = cleanItems(items);
    if (hint) set((s) => ({ hints: { ...s.hints, [sessionId]: hint } }));
  },
  clearHint: (sessionId) => set((s) => {
    if (!(sessionId in s.hints)) return s;
    const { [sessionId]: _gone, ...rest } = s.hints;
    return { hints: rest };
  }),
}));
