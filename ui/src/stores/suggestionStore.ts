/**
 * Suggestion buttons above the chat input, and the hint inside it.
 *
 * Three sources (doc/chat-spec.md § Suggestions): a skill's starters (read
 * from its frontmatter, not kept here), the page's buttons for what is on
 * screen, and the follow-ups the last reply offered. The first follow-up is
 * the input's grey hint — Tab takes it; the row belongs to the page.
 */
import { create } from 'zustand';

/** The row never shows more than this. */
export const MAX_SUGGESTIONS = 4;

interface SuggestionState {
  /** Set by the app page around this chat — it outlives a New chat. Empty
   *  hands the row back to the starters. */
  page: string[];
  /** Offered by each session's last reply; cleared when the person sends.
   *  A page change leaves them: they live in the input, the page in the row. */
  followups: Record<string, string[]>;
  setPage: (sessionId: string | null, items: unknown) => void;
  setFollowups: (sessionId: string, items: unknown) => void;
  clearFollowups: (sessionId: string) => void;
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

/** The input's grey hint: the reply's first follow-up, what they would
 *  most likely type next. */
export function inputHint(followups: string[] | undefined): string | undefined {
  return followups?.[0];
}

/** What the row shows: the page's, else the follow-ups the hint didn't
 *  take ([hinted]: the input shows the first), else the starters. */
export function visibleSuggestions(followups: string[] | undefined, page: string[], starters: string[] | undefined, hinted = true): string[] {
  if (page.length > 0) return page;
  if (followups && followups.length > 0) return hinted ? followups.slice(1) : followups;
  return cleanItems(starters);
}

export const useSuggestionStore = create<SuggestionState>((set) => ({
  page: [],
  followups: {},
  setPage: (_sessionId, items) => set({ page: cleanItems(items) }),
  setFollowups: (sessionId, items) => set((s) => ({
    followups: { ...s.followups, [sessionId]: cleanItems(items) },
  })),
  clearFollowups: (sessionId) => set((s) => ({
    followups: { ...s.followups, [sessionId]: [] },
  })),
}));
