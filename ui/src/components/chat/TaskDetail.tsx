/** A background task with no subagent chat behind it (a shell, a watch, a
 *  workflow): its whole command, how it stands, its latest output or how it
 *  ended, and where its full output is. */
import React from 'react';
import { STATUS_LABEL, elapsedOf, type PaneItem } from '../../lib/paneItems.mts';
import { useNow } from '../../hooks/useNow';
import { KindBadge } from './TaskList';

const STATUS_STYLE: Record<string, string> = {
  running: 'text-amber-600 dark:text-amber-400',
  completed: 'text-emerald-600 dark:text-emerald-400',
  failed: 'text-red-600 dark:text-red-400',
  stopped: 'text-slate-500',
  interrupted: 'text-slate-500',
};

const Field: React.FC<{ label: string; children: React.ReactNode }> = ({ label, children }) => (
  <div>
    <div className="text-[10px] uppercase tracking-wider text-slate-400 dark:text-slate-500 font-semibold mb-1">{label}</div>
    {children}
  </div>
);

export const TaskDetail: React.FC<{ item: PaneItem }> = ({ item }) => {
  const now = useNow(item.running);
  const task = item.task;
  const elapsed = elapsedOf(item, now);
  return (
    <div className="flex-1 min-h-0 overflow-y-auto custom-scrollbar px-3 py-3 flex flex-col gap-3 text-[12px]">
      <div className="flex items-center gap-1.5 text-[11px] text-slate-500 dark:text-slate-400">
        <KindBadge kind={item.kind} />
        <span className={STATUS_STYLE[item.status] ?? ''}>{STATUS_LABEL[item.status] ?? item.status}</span>
        {elapsed && <span className="tabular-nums">· {elapsed}</span>}
      </div>
      {task?.title && (
        <Field label={item.kind === 'shell' ? 'Command' : 'Task'}>
          <pre data-no-i18n className="font-mono text-[12px] whitespace-pre-wrap break-words bg-slate-100 dark:bg-white/10 text-slate-900 dark:text-slate-100 rounded-md px-2.5 py-1.5">{task.title}</pre>
        </Field>
      )}
      {task?.activity && (
        <Field label={item.running ? 'Latest output' : 'Last output'}>
          <pre data-no-i18n className="font-mono text-[12px] whitespace-pre-wrap break-words text-slate-700 dark:text-slate-300">{task.activity}</pre>
        </Field>
      )}
      {task?.summary && (
        <Field label="Summary">
          <div className="text-slate-800 dark:text-slate-200 whitespace-pre-wrap break-words" data-no-i18n>{task.summary}</div>
        </Field>
      )}
      {task?.output_file && (
        <Field label="Output file">
          <code data-no-i18n className="font-mono text-[11px] break-all select-all text-slate-600 dark:text-slate-300">{task.output_file}</code>
        </Field>
      )}
    </div>
  );
};
