import React, { useState } from 'react';

/** Claude Code-style thinking/loading spinner indicator. */
export const ThinkingIndicator: React.FC<{ text: string }> = ({ text }) => {
  const thinkingMatch = text.match(/^Thinking\s*\(([^)]+)\)$/i);
  const loadingMatch = text.match(/^(?:Loading model|Model loading)[:\s]*(.*)$/i);
  const label = thinkingMatch
    ? 'Thinking'
    : loadingMatch
      ? 'Loading model'
      : text.replace(/\.{2,}$/, '');
  const detail = thinkingMatch?.[1] || loadingMatch?.[1] || '';

  return (
    <div className="flex items-center gap-2 py-1">
      <span className="flex items-center gap-0.5">
        <span className="w-1.5 h-1.5 rounded-full bg-blue-500 animate-[thinking-bounce_1.4s_ease-in-out_0s_infinite]" />
        <span className="w-1.5 h-1.5 rounded-full bg-blue-500 animate-[thinking-bounce_1.4s_ease-in-out_0.2s_infinite]" />
        <span className="w-1.5 h-1.5 rounded-full bg-blue-500 animate-[thinking-bounce_1.4s_ease-in-out_0.4s_infinite]" />
      </span>
      <span className="text-[13px] text-slate-500 dark:text-slate-400 font-medium">
        {label}
        {detail && (
          <span className="font-normal text-slate-400 dark:text-slate-500 ml-1">({detail})</span>
        )}
      </span>
    </div>
  );
};

const SPINNER_VERBS = [
  'Thinking', 'Pondering', 'Brewing', 'Cogitating', 'Reticulating',
  'Combobulating', 'Noodling', 'Musing', 'Simmering', 'Percolating',
  'Ruminating', 'Contemplating', 'Marinating', 'Stewing', 'Conjuring',
  'Scheming', 'Tinkering', 'Crafting', 'Hatching', 'Computing',
  'Deliberating', 'Vibing', 'Spelunking', 'Meandering', 'Clauding', 'Frolicking',
];

const pickRandomVerb = () => SPINNER_VERBS[Math.floor(Math.random() * SPINNER_VERBS.length)];

/** Status spinner — `✶ Verb…` picks one random verb per appearance, stable until unmount. */
export const StatusSpinner: React.FC = () => {
  const [verb] = useState(pickRandomVerb);

  return (
    <div className="flex items-center gap-1.5 py-1 text-[13px] text-slate-500 dark:text-slate-400 font-medium animate-pulse">
      <span className="text-blue-500">✶</span>
      <span>{verb}…</span>
    </div>
  );
};
