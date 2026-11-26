/**
 * The memory a session started with, folded: the core rows and index the
 * system prompt was given. Persisted as a `memory-start` row (engine:
 * `core_block::start_note`) whose first line is `[memory-start core=N index=M]`
 * and whose rest is the exact text the model got. Same look as
 * MemoryRecallMessage.
 */
import React, { useState } from 'react';
import { Brain, ChevronDown, ChevronRight } from 'lucide-react';

const HEADER_RE = /^\[memory-start core=(\d+) index=(\d+)\]\n?/;

export const MemoryStartMessage: React.FC<{ text: string }> = ({ text }) => {
  const [expanded, setExpanded] = useState(false);
  const m = text.match(HEADER_RE);
  const body = m ? text.slice(m[0].length) : text;
  const label = m
    ? `Memory at session start: ${m[1]} core · ${m[2]} index`
    : 'Memory at session start';

  return (
    <div className="w-full flex justify-start">
      <div className="max-w-full text-[12px] leading-relaxed">
        <button
          onClick={() => setExpanded((v) => !v)}
          className="inline-flex items-center gap-1.5 px-2 py-0.5 rounded-md text-slate-500 dark:text-slate-400 hover:bg-slate-100 dark:hover:bg-white/[0.04] hover:text-slate-700 dark:hover:text-slate-200 transition-colors select-none"
          aria-expanded={expanded}
          title={expanded ? 'Hide start details' : 'Show start details'}
        >
          {expanded ? <ChevronDown size={11} /> : <ChevronRight size={11} />}
          <Brain size={12} className="opacity-70" />
          <span className="font-medium">{label}</span>
        </button>

        {expanded && (
          <div
            data-no-i18n
            className="mt-1 ml-5 border-l-2 border-slate-200 dark:border-white/10 pl-3 py-1 text-slate-700 dark:text-slate-300 break-words whitespace-pre-wrap"
          >
            {body}
          </div>
        )}
      </div>
    </div>
  );
};
