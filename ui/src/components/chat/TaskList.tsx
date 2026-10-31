/** The subagent pane's list view (doc/background-tasks-spec.md): every
 *  subagent and background task of the session, running ones on top, ended
 *  ones folded below. A row opens the item's detail; a running background
 *  task can be stopped from its row. */
import React, { useState } from 'react';
import { ChevronRight, Loader2, Square } from 'lucide-react';
import { cn } from '../../lib/cn';
import { KIND_LABEL, elapsedOf, type PaneItem } from '../../lib/paneItems.mts';
import { useNow } from '../../hooks/useNow';

/** What a running subagent or task shows wherever it is listed: a spinner, never a still dot. */
export const RunningSpinner: React.FC<{ size?: number; className?: string }> = ({ size = 11, className }) => (
  <Loader2 size={size} className={cn('shrink-0 animate-spin text-amber-500', className)} aria-label="running" />
);

export const StatusDot: React.FC<{ status: string; needsUser?: boolean; className?: string }> = ({ status, needsUser, className }) => (
  status === 'running' && !needsUser ? <RunningSpinner className={className} /> :
  <span className={cn('w-1.5 h-1.5 rounded-full shrink-0', className,
    needsUser ? 'bg-blue-500 animate-pulse'
      : status === 'running' ? 'bg-amber-500 animate-pulse'
        : status === 'failed' ? 'bg-red-500'
          : status === 'completed' ? 'bg-emerald-500' : 'bg-slate-400')} />
);

export const KindBadge: React.FC<{ kind: string }> = ({ kind }) => (
  <span className="shrink-0 text-[10px] font-semibold px-1 rounded bg-slate-200/70 dark:bg-white/10 text-slate-600 dark:text-slate-300">
    {KIND_LABEL[kind] ?? kind}
  </span>
);

export const StopButton: React.FC<{ onStop: () => void; title: string }> = ({ onStop, title }) => (
  <button
    onClick={(e) => { e.stopPropagation(); onStop(); }}
    className="shrink-0 inline-flex items-center gap-0.5 text-[10px] font-semibold px-1.5 py-0.5 rounded text-red-600 dark:text-red-400 hover:bg-red-500/10"
    title={title}
  >
    <Square size={9} fill="currentColor" /> Stop
  </button>
);

const TaskRow: React.FC<{
  item: PaneItem;
  now: number;
  needsUser: boolean;
  onOpen: () => void;
  onStopTask: (id: string) => void;
}> = ({ item, now, needsUser, onOpen, onStopTask }) => {
  const elapsed = elapsedOf(item, now);
  return (
    <div
      className="group flex items-start gap-2 px-2 py-1.5 text-[12px] border-t border-slate-100 dark:border-white/5 cursor-pointer hover:bg-white/70 dark:hover:bg-white/5"
      onClick={onOpen}
    >
      <StatusDot status={item.status} needsUser={needsUser} className="mt-[5px]" />
      <div className="min-w-0 flex-1">
        <div className="flex items-center gap-1.5">
          <KindBadge kind={item.kind} />
          <span className={cn('truncate text-slate-800 dark:text-slate-200', item.kind !== 'subagent' && 'font-mono')} data-no-i18n title={item.entry?.task || item.task?.title}>{item.title}</span>
          {item.agentType && item.agentType !== item.title && (
            <span className="shrink-0 font-mono text-[10px] text-slate-400 dark:text-slate-500" data-no-i18n>{item.agentType}</span>
          )}
        </div>
        <div className="flex items-center gap-1.5 text-[11px] text-slate-500 dark:text-slate-400">
          {elapsed && <span className="shrink-0 tabular-nums">{elapsed}</span>}
          {item.context && <span className="shrink-0 tabular-nums font-mono" title="Context window of its latest request">{item.context}</span>}
          {item.line && <span className="truncate" data-no-i18n>{elapsed || item.context ? `· ${item.line}` : item.line}</span>}
        </div>
      </div>
      {item.running && item.task && (
        <StopButton
          onStop={() => onStopTask(item.task!.id)}
          title="Stop this background task (the conversation and other tasks keep going)"
        />
      )}
    </div>
  );
};

export const TaskList: React.FC<{
  items: PaneItem[];
  /** Key of the subagent waiting on the person's answer, if any. */
  askingKey?: string | null;
  onOpen: (key: string) => void;
  onStopTask: (id: string) => void;
}> = ({ items, askingKey, onOpen, onStopTask }) => {
  const [showEnded, setShowEnded] = useState(false);
  const running = items.filter((i) => i.running);
  const ended = items.filter((i) => !i.running);
  const now = useNow(running.length > 0);
  const row = (item: PaneItem) => (
    <TaskRow
      key={item.key}
      item={item}
      now={now}
      needsUser={item.key === askingKey}
      onOpen={() => onOpen(item.key)}
      onStopTask={onStopTask}
    />
  );
  const groupLabel = 'px-2 py-1.5 flex items-center gap-1 text-[10px] uppercase tracking-wider text-slate-400 dark:text-slate-500 font-semibold';
  return (
    <div className="flex-1 min-h-0 overflow-y-auto custom-scrollbar">
      <div className={groupLabel}>
        <span>Running</span>
        <span className="tabular-nums">{running.length}</span>
      </div>
      {running.map(row)}
      {running.length === 0 && (
        <div className="px-2 pb-2 text-[11px] text-slate-400 dark:text-slate-500">Nothing running</div>
      )}
      {ended.length > 0 && (<>
        <button
          onClick={() => setShowEnded((v) => !v)}
          className={cn(groupLabel, 'w-full border-t border-slate-200 dark:border-white/10 hover:text-slate-600 dark:hover:text-slate-300')}
        >
          <ChevronRight size={11} className={cn('transition-transform', showEnded && 'rotate-90')} />
          <span>Ended</span>
          <span className="tabular-nums">{ended.length}</span>
        </button>
        {showEnded && ended.map(row)}
      </>)}
    </div>
  );
};
