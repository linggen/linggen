/**
 * Renders a per-turn auto-recall message inline in the chat timeline.
 *
 * Backend (`server/chat/runtime.rs::push_user_turn_with_recall`) persists
 * recall hits as a `ChatMsg` with `from_id="memory"`; this component
 * picks those up and renders them as a collapsible widget so the user
 * can see exactly what context the model received that turn.
 *
 * Content shape (one row per line, format mirrors CC's `recall.sh`):
 *   From memory (<type>, <host>, <YYYY-MM-DD>, id=<uuid>): <content>
 *   ...
 *   Note: ... (optional reconcile footer when ≥2 rows)
 *
 * Lines that don't match the prefix become a free-text trailer (the
 * footer, or any future format the backend emits).
 */
import React, { useMemo, useState } from 'react';
import { Brain, ChevronDown, ChevronRight } from 'lucide-react';

interface ParsedRow {
  type: string;
  host: string;
  date: string;
  id: string;
  content: string;
}

interface ParsedRecall {
  rows: ParsedRow[];
  trailer: string;
}

const ROW_RE = /^From memory \(([^,]+), ([^,]+), ([^,]+), id=([^)]+)\): (.+)$/;

function parseRecallText(text: string): ParsedRecall {
  const rows: ParsedRow[] = [];
  const trailerLines: string[] = [];
  for (const line of text.split('\n')) {
    const trimmed = line.trim();
    if (!trimmed) continue;
    const m = trimmed.match(ROW_RE);
    if (m) {
      rows.push({ type: m[1], host: m[2], date: m[3], id: m[4], content: m[5] });
    } else {
      trailerLines.push(trimmed);
    }
  }
  return { rows, trailer: trailerLines.join('\n') };
}

export const MemoryRecallMessage: React.FC<{ text: string }> = ({ text }) => {
  const parsed = useMemo(() => parseRecallText(text), [text]);
  const [expanded, setExpanded] = useState(false);

  if (parsed.rows.length === 0 && !parsed.trailer) return null;

  const count = parsed.rows.length;
  const label = count === 0
    ? 'memory note'
    : count === 1
      ? '1 memory recalled'
      : `${count} memories recalled`;

  return (
    <div className="w-full flex justify-start">
      <div className="max-w-full text-[12px] leading-relaxed">
        <button
          onClick={() => setExpanded((v) => !v)}
          className="inline-flex items-center gap-1.5 px-2 py-0.5 rounded-md text-slate-500 dark:text-slate-400 hover:bg-slate-100 dark:hover:bg-white/[0.04] hover:text-slate-700 dark:hover:text-slate-200 transition-colors select-none"
          aria-expanded={expanded}
          title={expanded ? 'Hide recall details' : 'Show recall details'}
        >
          {expanded ? <ChevronDown size={11} /> : <ChevronRight size={11} />}
          <Brain size={12} className="opacity-70" />
          <span className="font-medium">{label}</span>
        </button>

        {expanded && (
          <div className="mt-1 ml-5 space-y-1.5 border-l-2 border-slate-200 dark:border-white/10 pl-3 py-1">
            {parsed.rows.map((row) => (
              <div key={row.id} className="space-y-0.5">
                <div className="flex items-center gap-1.5 text-[10px] text-slate-400 dark:text-slate-500 tabular-nums">
                  <span className="font-medium uppercase tracking-wide">{row.type}</span>
                  <span>·</span>
                  <span>{row.host}</span>
                  <span>·</span>
                  <span>{row.date}</span>
                  <span>·</span>
                  <button
                    onClick={(e) => {
                      e.stopPropagation();
                      navigator.clipboard?.writeText(row.id).catch(() => {});
                    }}
                    className="font-mono hover:text-slate-600 dark:hover:text-slate-300"
                    title={`Click to copy id\n${row.id}`}
                  >
                    {row.id.slice(0, 8)}
                  </button>
                </div>
                <div className="text-slate-700 dark:text-slate-300 break-words">
                  {row.content}
                </div>
              </div>
            ))}
            {parsed.trailer && (
              <div className="pt-1 mt-1 border-t border-slate-200/60 dark:border-white/[0.06] text-[10px] text-slate-400 dark:text-slate-500 italic whitespace-pre-wrap">
                {parsed.trailer}
              </div>
            )}
          </div>
        )}
      </div>
    </div>
  );
};
