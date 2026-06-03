import React from 'react';

/**
 * Inline notice shown when the engine AUTO-compacts the conversation
 * (summarizes older messages to free context). Rendered for chat messages
 * tagged `from === 'compaction'`, injected by the context_usage event handler
 * when the engine reports `compressed`. Without this, auto-compaction is
 * silent — and it can quietly drop tool results (e.g. fetched threads) the
 * agent still needs, with zero on-screen signal. Mirrors MemoryRecallMessage.
 */
export const CompactionMessage: React.FC<{ text: string }> = ({ text }) => {
  return (
    <div className="w-full flex items-center gap-2 my-1.5 select-none" title={text}>
      <div className="h-px flex-1 bg-slate-200 dark:bg-white/10" />
      <span className="text-[10px] uppercase tracking-wider text-slate-400 dark:text-slate-500 whitespace-nowrap">
        ⊙ {text}
      </span>
      <div className="h-px flex-1 bg-slate-200 dark:bg-white/10" />
    </div>
  );
};
