/**
 * Suggestion buttons above the chat input.
 *
 * Three sources; the latest to change wins (doc/chat-spec.md § Suggestions):
 * a skill's starters (read from its frontmatter, not kept here), the page's
 * buttons for what is on screen, and the follow-ups the last reply offered.
 */
import { create } from 'zustand';

/** The row never shows more than this. */
export const MAX_SUGGESTIONS = 4;

interface SuggestionState {
  /** Set by the app page around this chat — it outlives a New chat. Empty
   *  hands the row back to the starters. */
  page: string[];
  /** Offered by each session's last reply; cleared when the person sends. */
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

/** What the row shows: follow-ups, else the page's, else the starters. */
export function visibleSuggestions(followups: string[] | undefined, page: string[], starters: string[] | undefined): string[] {
  if (followups && followups.length > 0) return followups;
  if (page.length > 0) return page;
  return cleanItems(starters);
}

export const useSuggestionStore = create<SuggestionState>((set) => ({
  page: [],
  followups: {},
  // A page change is the newest thing on screen — it replaces the follow-ups.
  setPage: (sessionId, items) => set((s) => ({
    page: cleanItems(items),
    followups: sessionId ? { ...s.followups, [sessionId]: [] } : s.followups,
  })),
  setFollowups: (sessionId, items) => set((s) => ({
    followups: { ...s.followups, [sessionId]: cleanItems(items) },
  })),
  clearFollowups: (sessionId) => set((s) => ({
    followups: { ...s.followups, [sessionId]: [] },
  })),
}));
