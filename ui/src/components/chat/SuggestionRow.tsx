/**
 * Suggestion buttons above the chat input — a tap sends the words as the
 * person's own message. See doc/chat-spec.md § Suggestions.
 */
import React from 'react';
import { useSessionStore } from '../../stores/sessionStore';
import { useSuggestionStore, visibleSuggestions } from '../../stores/suggestionStore';
import type { SkillInfo } from '../../types';

interface SuggestionRowProps {
  skills: SkillInfo[];
  /** While they type or a turn runs, the row steps aside. */
  hidden: boolean;
  /** The input shows the next-prompt hint (not on a phone). */
  hinted: boolean;
  onPick: (text: string) => void;
}

export const SuggestionRow: React.FC<SuggestionRowProps> = ({ skills, hidden, hinted, onPick }) => {
  const sessionId = useSessionStore((s) => s.activeSessionId);
  const skillName = useSessionStore((s) => s.activeSkillName);
  const hint = useSuggestionStore((s) => (sessionId ? s.hints[sessionId] : undefined));
  const page = useSuggestionStore((s) => s.page);
  const starters = skillName ? skills.find((sk) => sk.name === skillName)?.suggestions : undefined;
  const items = visibleSuggestions(hint, page, starters, hinted);
  if (hidden || items.length === 0) return null;

  return (
    <div className="flex flex-wrap gap-1.5 px-0.5" role="group" aria-label="Suggested messages">
      {items.map((text) => (
        <button
          key={text}
          type="button"
          title={text}
          onClick={() => onPick(text)}
          className="max-w-full truncate text-[12px] px-2.5 py-1 rounded-full border border-slate-300/80 dark:border-white/10 bg-white dark:bg-white/5 text-slate-600 dark:text-slate-300 hover:border-blue-400 hover:text-blue-600 dark:hover:border-blue-500/40 dark:hover:text-blue-400 transition-colors"
        >
          {text}
        </button>
      ))}
    </div>
  );
};
