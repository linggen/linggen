/**
 * SubagentPane — dedicated side panel showing live subagent runs for the
 * current session. Sits next to (or below) the main chat in ChatPanel.
 *
 * Layout:
 *   ┌─ Tabs (one per subagent, ●/✓/❓ status) ─┐
 *   │  task header                              │
 *   │  tool calls (existing SubagentTreeView)   │
 *   │  result text on done                      │
 *   └───────────────────────────────────────────┘
 *
 * Visibility: rendered only when ≥1 subagent has been registered for the
 * current session. ChatPanel decides whether to mount this or fall back
 * to the inline SubagentTreeView (the iframe-friendly path).
 */
import React, { useEffect, useMemo, useState } from 'react';
import { cn } from '../../lib/cn';
import type { ChatMessage, SubagentTreeEntry } from '../../types';
import { SubagentTreeView } from './SubagentTreeView';

interface Props {
  messages: ChatMessage[];
  /** Toggled by ChatPanel's auto-collapse timer; lets the parent hide
   *  the pane after the last subagent has been done for a few seconds. */
  visible: boolean;
}

/** Collect every subagent entry attached to any message in this session,
 *  flattened in chronological order. Each parent message owns its own
 *  `subagentTree`; we surface them all in one tab strip. */
function collectEntries(messages: ChatMessage[]): SubagentTreeEntry[] {
  const out: SubagentTreeEntry[] = [];
  for (const msg of messages) {
    if (msg.subagentTree && msg.subagentTree.length > 0) {
      out.push(...msg.subagentTree);
    }
  }
  return out;
}

export const SubagentPane: React.FC<Props> = ({ messages, visible }) => {
  const entries = useMemo(() => collectEntries(messages), [messages]);

  // Default active tab: most recently registered subagent.
  const [activeId, setActiveId] = useState<string | null>(null);
  useEffect(() => {
    if (entries.length === 0) {
      setActiveId(null);
      return;
    }
    if (!activeId || !entries.some((e) => e.subagentId === activeId)) {
      // Prefer a running one; otherwise the latest.
      const running = entries.find((e) => e.status === 'running');
      const fallback = entries[entries.length - 1];
      setActiveId((running ?? fallback).subagentId);
    }
  }, [entries, activeId]);

  if (!visible || entries.length === 0) return null;

  const active =
    entries.find((e) => e.subagentId === activeId) ?? entries[entries.length - 1];

  return (
    <div className="flex flex-col min-h-0 border-l border-slate-200 dark:border-white/10 bg-slate-50/60 dark:bg-[#0b0b0b]">
      {/* Tab strip */}
      <div className="flex items-center gap-1 px-2 py-1.5 border-b border-slate-200 dark:border-white/10 overflow-x-auto custom-scrollbar shrink-0">
        <span className="text-[10px] uppercase tracking-wider text-slate-400 dark:text-slate-500 font-semibold mr-1 shrink-0">
          Subagents
        </span>
        {entries.map((entry) => {
          const isActive = entry.subagentId === active.subagentId;
          const glyph =
            entry.status === 'running'
              ? '●'
              : entry.status === 'failed'
                ? '✗'
                : '✓';
          const glyphColor =
            entry.status === 'running'
              ? 'text-amber-500'
              : entry.status === 'failed'
                ? 'text-red-500'
                : 'text-emerald-500';
          return (
            <button
              key={entry.subagentId}
              onClick={() => setActiveId(entry.subagentId)}
              className={cn(
                'px-2 py-1 text-[11px] rounded-md font-mono whitespace-nowrap transition-colors',
                isActive
                  ? 'bg-white dark:bg-[#1a1a1a] shadow-sm text-slate-900 dark:text-slate-100 ring-1 ring-slate-200 dark:ring-white/10'
                  : 'text-slate-500 dark:text-slate-400 hover:bg-white/70 dark:hover:bg-white/5',
              )}
              title={entry.task || entry.agentName}
            >
              <span
                className={cn(
                  'mr-1',
                  glyphColor,
                  entry.status === 'running' && 'animate-pulse',
                )}
              >
                {glyph}
              </span>
              {entry.agentName || entry.subagentId}
            </button>
          );
        })}
      </div>

      {/* Active tab content */}
      <div className="flex-1 overflow-y-auto px-3 py-2 custom-scrollbar min-h-0">
        {active.task && (
          <div className="text-[11px] text-slate-500 dark:text-slate-400 italic mb-2 whitespace-pre-wrap break-words line-clamp-3">
            Task: {active.task}
          </div>
        )}
        <SubagentTreeView
          entries={[active]}
          isGenerating={active.status === 'running'}
          isExpanded={true}
          onToggle={() => {
            /* always expanded in dedicated pane */
          }}
        />
      </div>
    </div>
  );
};
