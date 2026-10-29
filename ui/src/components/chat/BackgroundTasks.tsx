/** The session's background tasks (doc/background-tasks-spec.md): what each
 *  is, how long it has run, its latest output, how it ended. A running one can
 *  be stopped here; a background subagent opens its tab in the pane. */
import React, { useEffect, useState } from 'react';
import { Square, X } from 'lucide-react';
import { cn } from '../../lib/cn';
import type { BackgroundTask } from '../../types';

const KIND_LABEL: Record<string, string> = { shell: 'Shell', subagent: 'Agent', watch: 'Watch', workflow: 'Workflow' };

const STATUS_STYLE: Record<string, string> = {
  running: 'text-amber-600 dark:text-amber-400',
  completed: 'text-emerald-600 dark:text-emerald-400',
  failed: 'text-red-600 dark:text-red-400',
  stopped: 'text-slate-500',
  interrupted: 'text-slate-500',
};

export function formatElapsed(ms: number): string {
  const s = Math.max(0, Math.floor(ms / 1000));
  if (s < 60) return `${s}s`;
  const m = Math.floor(s / 60);
  if (m < 60) return `${m}m ${s % 60}s`;
  return `${Math.floor(m / 60)}h ${m % 60}m`;
}

/** Re-render once a second while anything runs, so elapsed times tick. */
function useNow(active: boolean): number {
  const [now, setNow] = useState(() => Date.now());
  useEffect(() => {
    if (!active) return;
    const t = setInterval(() => setNow(Date.now()), 1000);
    return () => clearInterval(t);
  }, [active]);
  return now;
}

export const BackgroundTasks: React.FC<{
  tasks: BackgroundTask[];
  onStop: (id: string) => void;
  onOpenSubagent?: (subagentId: string) => void;
  /** Close the pane, when no subagent tab strip above carries the button. */
  onClose?: () => void;
}> = ({ tasks, onStop, onOpenSubagent, onClose }) => {
  const running = tasks.filter((t) => t.status === 'running').length;
  const now = useNow(running > 0);
  if (tasks.length === 0) return null;
  return (
    <div className="border-b border-slate-200 dark:border-white/10 shrink-0 max-h-[45%] overflow-y-auto custom-scrollbar">
      <div className="flex items-center px-2 py-1.5 text-[10px] uppercase tracking-wider text-slate-400 dark:text-slate-500 font-semibold">
        <span className="flex-1">{running > 0 ? `Background · ${running} running` : 'Background'}</span>
        {onClose && (
          <button onClick={onClose} className="p-0.5 rounded hover:bg-slate-200/70 dark:hover:bg-white/10" title="Close">
            <X size={12} />
          </button>
        )}
      </div>
      {tasks.map((task) => {
        const isRunning = task.status === 'running';
        const elapsed = (task.ended_at ?? now) - task.started_at;
        const line = isRunning ? task.activity : task.summary;
        const opensTab = task.subagent_id && onOpenSubagent;
        return (
          <div
            key={task.id}
            className={cn('group flex items-start gap-2 px-2 py-1.5 text-[12px] border-t border-slate-100 dark:border-white/5 first:border-t-0',
              opensTab && 'cursor-pointer hover:bg-white/70 dark:hover:bg-white/5')}
            onClick={opensTab ? () => onOpenSubagent!(task.subagent_id!) : undefined}
            title={task.output_file ? `Output: ${task.output_file}` : undefined}
          >
            <span className={cn('mt-[3px] w-1.5 h-1.5 rounded-full shrink-0',
              isRunning ? 'bg-amber-500 animate-pulse' : task.status === 'failed' ? 'bg-red-500' : task.status === 'completed' ? 'bg-emerald-500' : 'bg-slate-400')} />
            <div className="min-w-0 flex-1">
              <div className="flex items-center gap-1.5">
                <span className="shrink-0 text-[10px] font-semibold px-1 rounded bg-slate-200/70 dark:bg-white/10 text-slate-600 dark:text-slate-300">
                  {KIND_LABEL[task.kind] ?? task.kind}
                </span>
                <span className="truncate font-mono text-slate-800 dark:text-slate-200">{task.title || task.id}</span>
              </div>
              <div className="flex items-center gap-1.5 text-[11px] text-slate-500 dark:text-slate-400">
                <span className={STATUS_STYLE[task.status] ?? ''}>{task.status}</span>
                <span>· {formatElapsed(elapsed)}</span>
                {line && <span className="truncate">· {line}</span>}
              </div>
            </div>
            {isRunning && (
              <button
                onClick={(e) => { e.stopPropagation(); onStop(task.id); }}
                className="shrink-0 inline-flex items-center gap-0.5 text-[10px] font-semibold px-1.5 py-0.5 rounded text-red-600 dark:text-red-400 hover:bg-red-500/10"
                title="Stop this background task (the conversation and other tasks keep going)"
              >
                <Square size={9} fill="currentColor" /> Stop
              </button>
            )}
          </div>
        );
      })}
    </div>
  );
};
