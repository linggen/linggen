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
import { X } from 'lucide-react';
import { cn } from '../../lib/cn';
import type { ChatMessage, ContentBlock, PendingAskUser, AskUserAnswer, SubagentTreeEntry } from '../../types';
import { MarkdownContent } from './MarkdownContent';
import { ContentBlockView } from './ContentBlockView';
import { AskUserCard } from '../AskUserCard';
import { ToolPermissionCard } from '../ToolPermissionCard';

interface Props {
  messages: ChatMessage[];
  /** Toggled by ChatPanel's auto-collapse timer; lets the parent hide
   *  the pane after the last subagent has been done for a few seconds. */
  visible: boolean;
  /** Pending AskUser (or permission) widget the engine emitted. Pass
   *  through only when the event's agent_id matches one of our tabs —
   *  ChatPanel does the predicate; we just render in the matching tab. */
  pendingAskUser?: PendingAskUser | null;
  onRespondToAskUser?: (questionId: string, answers: AskUserAnswer[]) => void;
  /** Cancel a running subagent. Wired to the same endpoint the main
   *  agent's cancel button uses; the subagent's tracking id is the run
   *  id the cancel API expects. */
  onCancelAgentRun?: (runId: string) => void | Promise<void>;
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

export const SubagentPane: React.FC<Props> = ({
  messages,
  visible,
  pendingAskUser,
  onRespondToAskUser,
  onCancelAgentRun,
}) => {
  const entries = useMemo(() => collectEntries(messages), [messages]);

  // Match the pending AskUser to one of our tabs. Prefer an exact
  // subagentId (run id) match — that's globally unique. Fall back to
  // an agentName match, preferring a *running* entry, so when two
  // subagents share the same name only the live one gets the widget.
  const askUserEntry = useMemo(() => {
    if (!pendingAskUser) return null;
    const id = pendingAskUser.agentId;
    const byRunId = entries.find((e) => e.subagentId === id);
    if (byRunId) return byRunId;
    const byNameRunning = entries.find(
      (e) => e.agentName === id && e.status === 'running',
    );
    if (byNameRunning) return byNameRunning;
    return entries.find((e) => e.agentName === id) ?? null;
  }, [entries, pendingAskUser]);

  // Default active tab: most recently registered subagent. Auto-switch
  // to the tab that owns the pending question.
  const [activeId, setActiveId] = useState<string | null>(null);
  useEffect(() => {
    if (entries.length === 0) {
      setActiveId(null);
      return;
    }
    if (askUserEntry) {
      setActiveId(askUserEntry.subagentId);
      return;
    }
    if (!activeId || !entries.some((e) => e.subagentId === activeId)) {
      // Prefer a running one; otherwise the latest.
      const running = entries.find((e) => e.status === 'running');
      const fallback = entries[entries.length - 1];
      setActiveId((running ?? fallback).subagentId);
    }
  }, [entries, activeId, askUserEntry]);

  if (!visible || entries.length === 0) return null;

  const active =
    entries.find((e) => e.subagentId === activeId) ?? entries[entries.length - 1];

  return (
    <div className="flex flex-col h-full min-h-0 border-l border-slate-200 dark:border-white/10 bg-slate-50/60 dark:bg-[#0b0b0b]">
      {/* Tab strip */}
      <div className="flex items-center gap-1 px-2 py-1.5 border-b border-slate-200 dark:border-white/10 overflow-x-auto custom-scrollbar shrink-0">
        <span className="text-[10px] uppercase tracking-wider text-slate-400 dark:text-slate-500 font-semibold mr-1 shrink-0">
          Subagents
        </span>
        {entries.map((entry) => {
          const isActive = entry.subagentId === active.subagentId;
          const needsUser =
            askUserEntry && askUserEntry.subagentId === entry.subagentId;
          const glyph = needsUser
            ? '❓'
            : entry.status === 'running'
              ? '●'
              : entry.status === 'failed'
                ? '✗'
                : '✓';
          const glyphColor = needsUser
            ? 'text-blue-500'
            : entry.status === 'running'
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

      {/* Active tab content — chat-style render matching main chat:
       *   user bubble (task) → tool blocks → assistant text (result)
       * Tool calls are also shown in the parent's main-chat tree, but
       * here they render with the same ⏺/⎿/✓ ContentBlockView used in
       * main chat (markdown, args/output expansion, etc.) so the pane
       * IS the full conversation thread between main agent and subagent.
       */}
      <div className="flex-1 overflow-y-auto px-3 py-3 custom-scrollbar min-h-0 flex flex-col gap-3">
        {/* Header row with optional stop button */}
        {active.status === 'running' && onCancelAgentRun && (
          <div className="flex items-center justify-end -mb-1">
            <button
              onClick={() => onCancelAgentRun(active.subagentId)}
              className="p-1 rounded text-slate-400 hover:text-red-500 hover:bg-red-500/10 transition-colors text-[11px]"
              title="Stop this subagent"
            >
              <X size={12} />
            </button>
          </div>
        )}

        {/* 1. Incoming task — user-style bubble (right-aligned, same as
         *    main chat's user bubbles). MarkdownContent handles the
         *    code blocks / lists / inline code in encoder prompts. */}
        {active.task && (
          <div className="self-start max-w-[92%]">
            <div className="text-[10px] uppercase tracking-wider text-slate-400 dark:text-slate-500 font-semibold mb-1">
              From main → {active.agentName || active.subagentId}
            </div>
            <div className="bg-slate-100 dark:bg-white/10 text-slate-900 dark:text-slate-100 rounded-md px-2.5 py-1.5 text-[13px]">
              <MarkdownContent text={active.task} />
            </div>
          </div>
        )}

        {/* 2. Tool calls — same ContentBlockView main chat uses, so the
         *    bubble style, expansion, args/output render are identical. */}
        {active.toolSteps && active.toolSteps.length > 0 && (
          <div className="flex flex-col gap-1">
            {active.toolSteps.map((step, idx) => {
              const block: ContentBlock = {
                type: 'tool_use',
                tool: step.toolName,
                args: step.args,
                status: step.status,
              };
              return (
                <ContentBlockView
                  key={idx}
                  block={block}
                  isLast={idx === active.toolSteps.length - 1}
                />
              );
            })}
          </div>
        )}

        {/* 3. Live thinking indicator while the subagent is mid-call
         *    with no tool yet — matches main chat's "Thinking…" line. */}
        {active.status === 'running' &&
          (!active.toolSteps || active.toolSteps.length === 0) &&
          active.currentActivity && (
            <div className="flex items-center gap-1.5 text-[12px] text-slate-500 dark:text-slate-400 italic">
              <span className="text-blue-500 animate-pulse">✶</span>
              <span>{active.currentActivity}</span>
            </div>
          )}

        {/* 4. AskUser widget — same card main chat uses, rendered here
         *    when the question's agent_id matches this tab. */}
        {askUserEntry &&
          askUserEntry.subagentId === active.subagentId &&
          pendingAskUser &&
          onRespondToAskUser && (
            <div>
              {pendingAskUser.questions[0]?.header === 'Permission' ? (
                <ToolPermissionCard
                  pending={pendingAskUser}
                  onRespond={onRespondToAskUser}
                />
              ) : (
                <AskUserCard
                  pending={pendingAskUser}
                  onRespond={onRespondToAskUser}
                />
              )}
            </div>
          )}

        {/* 5. Final result — assistant-style text bubble (left-aligned,
         *    same MD treatment as main chat agent replies). */}
        {active.resultText && (
          <div className="self-start max-w-[92%]">
            <div className="text-[10px] uppercase tracking-wider text-slate-400 dark:text-slate-500 font-semibold mb-1">
              {active.agentName || active.subagentId} → main
            </div>
            <div className="text-slate-800 dark:text-slate-200 text-[13px]">
              <MarkdownContent text={active.resultText} />
            </div>
          </div>
        )}
      </div>
    </div>
  );
};
