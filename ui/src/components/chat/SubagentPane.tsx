/**
 * SubagentPane — side panel beside (or below) the main chat in ChatPanel,
 * listing every subagent and background task of the current session.
 *
 * One surface, list → detail:
 *   ┌─ Subagents & tasks · 2 running ──── ▣ ✕ ┐   ┌─ ← Back  title ──── ■ ⧉ ✕ ┐
 *   │  Running: one row per item               │ → │  subagent chat, or a      │
 *   │  ▸ Ended (folded)                        │   │  shell's command/output   │
 *   └──────────────────────────────────────────┘   └───────────────────────────┘
 *
 * Visibility: ChatPanel mounts this while the pane is open, so each
 * opening starts at the list; a new subagent never pulls the person out of
 * the detail they're reading.
 */
import React, { useEffect, useMemo, useRef, useState } from 'react';
import { X, Copy, FileText, ArrowLeft } from 'lucide-react';
import type { BackgroundTask, ChatMessage, PendingAskUser, AskUserAnswer } from '../../types';
import { paneItems, subagentTranscript, type PaneItem } from '../../lib/paneItems.mts';
import { TaskList, StopButton, RunningSpinner } from './TaskList';
import { TaskDetail } from './TaskDetail';
import { SubagentDetail } from './SubagentDetail';

interface Props {
  messages: ChatMessage[];
  /** Toggled by ChatPanel's auto-collapse timer; lets the parent hide
   *  the pane after the last subagent has been done for a while. */
  visible: boolean;
  /** Pending AskUser (or permission) widget the engine emitted. Passed
   *  through only when its agent_id is one of our subagents — ChatPanel
   *  does the predicate; we render it in the matching detail. */
  pendingAskUser?: PendingAskUser | null;
  onRespondToAskUser?: (questionId: string, answers: AskUserAnswer[]) => void;
  /** Cancel a running foreground subagent. Wired to the same endpoint the
   *  main agent's cancel button uses; the subagent's tracking id is the run
   *  id the cancel API expects. */
  onCancelAgentRun?: (runId: string) => void | Promise<void>;
  /** Close the entire pane immediately (overrides auto-collapse). */
  onClose?: () => void;
  /** The session's background tasks, merged into the list with the subagents. */
  tasks?: BackgroundTask[];
  onStopTask?: (id: string) => void;
  /** Told whether an item's detail is open: the pane does not close itself
   *  under someone reading. */
  onReadingChange?: (reading: boolean) => void;
}

const iconButton = 'p-1 rounded text-slate-400 hover:text-blue-500 hover:bg-blue-500/10 transition-colors';

export const SubagentPane: React.FC<Props> = ({
  messages,
  visible,
  pendingAskUser,
  onRespondToAskUser,
  onCancelAgentRun,
  onClose,
  tasks = [],
  onStopTask,
  onReadingChange,
}) => {
  const entries = useMemo(() => messages.flatMap((m) => m.subagentTree ?? []), [messages]);
  const items = useMemo(() => paneItems(entries, tasks), [entries, tasks]);
  const running = items.filter((i) => i.running).length;

  // Match the pending AskUser to one of our subagents. Prefer an exact
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

  // The open item, or null for the list. A question opens the subagent that
  // asked it, once per question: the widget lives in its detail.
  const [openKey, setOpenKey] = useState<string | null>(null);
  const shownQuestion = useRef<string | null>(null);
  useEffect(() => {
    if (!askUserEntry || !pendingAskUser) return;
    if (shownQuestion.current === pendingAskUser.questionId) return;
    shownQuestion.current = pendingAskUser.questionId;
    setOpenKey(askUserEntry.subagentId);
  }, [askUserEntry, pendingAskUser]);

  // An item that is gone (a session switch, Clear) falls back to the list.
  const open = items.find((i) => i.key === openKey) ?? null;
  const isReading = open !== null;
  useEffect(() => {
    onReadingChange?.(isReading);
    return () => onReadingChange?.(false);
  }, [isReading, onReadingChange]);

  if (!visible || items.length === 0) return null;

  return (
    <div className="flex flex-col h-full min-h-0 border-l border-slate-200 dark:border-white/10 bg-slate-50/60 dark:bg-[#0b0b0b]">
      <div className="flex items-center gap-1 px-2 py-1.5 border-b border-slate-200 dark:border-white/10 shrink-0">
        {open ? (
          <DetailHeader
            item={open}
            onBack={() => setOpenKey(null)}
            onStopTask={onStopTask}
            onCancelAgentRun={onCancelAgentRun}
          />
        ) : (
          <span className="flex-1 min-w-0 truncate text-[10px] uppercase tracking-wider text-slate-400 dark:text-slate-500 font-semibold">
            {running > 0 ? `Subagents & tasks · ${running} running` : 'Subagents & tasks'}
          </span>
        )}
        {onClose && (
          <button
            onClick={onClose}
            className="p-1 rounded text-slate-400 hover:text-red-500 hover:bg-red-500/10 transition-colors"
            title="Close pane"
            aria-label="Close pane"
          >
            <X size={12} />
          </button>
        )}
      </div>

      {!open && (
        <TaskList
          items={items}
          askingKey={askUserEntry?.subagentId}
          onOpen={setOpenKey}
          onStopTask={(id) => onStopTask?.(id)}
        />
      )}
      {open?.entry && (
        <SubagentDetail
          key={open.key}
          entry={open.entry}
          pendingAskUser={askUserEntry?.subagentId === open.key ? pendingAskUser : null}
          onRespondToAskUser={onRespondToAskUser}
        />
      )}
      {open && !open.entry && <TaskDetail key={open.key} item={open} />}
    </div>
  );
};

/** The detail's top row: back to the list, the item's title, Stop while it
 *  runs, and a subagent's copy actions. */
const DetailHeader: React.FC<{
  item: PaneItem;
  onBack: () => void;
  onStopTask?: (id: string) => void;
  onCancelAgentRun?: (runId: string) => void | Promise<void>;
}> = ({ item, onBack, onStopTask, onCancelAgentRun }) => {
  // A background task stops through the task endpoint; a foreground
  // subagent cancels its run.
  const stop = !item.running ? null
    : item.task && onStopTask ? () => onStopTask(item.task!.id)
      : item.entry && onCancelAgentRun ? () => onCancelAgentRun(item.entry!.subagentId)
        : null;
  const entry = item.entry;
  return (
    <>
      <button
        onClick={onBack}
        className="shrink-0 inline-flex items-center gap-0.5 px-1 py-0.5 rounded text-[11px] text-slate-500 dark:text-slate-400 hover:text-slate-800 dark:hover:text-slate-200 hover:bg-white/70 dark:hover:bg-white/5"
        title="Back to the list"
      >
        <ArrowLeft size={12} /> Back
      </button>
      {item.running && <RunningSpinner size={12} />}
      <span
        className="flex-1 min-w-0 truncate text-[12px] font-medium text-slate-800 dark:text-slate-200"
        title={entry?.task || item.task?.title}
        data-no-i18n
      >
        {item.title}
      </span>
      {item.context && (
        <span className="shrink-0 text-[10px] font-mono tabular-nums text-slate-400" title="Context window of its latest request">{item.context}</span>
      )}
      {stop && (
        <StopButton
          onStop={stop}
          title={item.task ? 'Stop this background task (the conversation and other tasks keep going)' : 'Stop this subagent'}
        />
      )}
      {entry && (<>
        <button
          onClick={() => navigator.clipboard.writeText(subagentTranscript(entry))}
          className={iconButton}
          title="Copy chat transcript"
          aria-label="Copy chat"
        >
          <Copy size={12} />
        </button>
        <button
          // Copies the raw task prompt — the system-prompt-equivalent for
          // this subagent run. Useful for debugging or rerunning the
          // encoder against the same input.
          onClick={() => navigator.clipboard.writeText(entry.task || '')}
          className={iconButton}
          title="Copy system prompt (task)"
          aria-label="Copy system prompt"
        >
          <FileText size={12} />
        </button>
      </>)}
    </>
  );
};
