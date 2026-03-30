import React from 'react';
import { cn } from '../../lib/cn';
import type { SubagentTreeEntry } from '../../types';
import { formatCompactTokens } from './utils/activity';
import { truncateDetail } from './utils/content-block';

/** Subagent tree view — Claude Code-style per-entry Task() blocks. */
export const SubagentTreeView: React.FC<{
  entries: SubagentTreeEntry[];
  isGenerating: boolean;
  isExpanded: boolean;
  onToggle: () => void;
}> = ({ entries, isGenerating, isExpanded, onToggle }) => {
  const allDone = entries.every((e) => e.status !== 'running');

  const showExpanded = isGenerating || isExpanded;

  return (
    <div
      className={cn('mb-1.5 font-mono text-[12px]', !isGenerating && allDone && 'cursor-pointer select-none')}
      onClick={!isGenerating && allDone ? onToggle : undefined}
    >
      {entries.map((entry) => {
        const isRunning = entry.status === 'running';
        const bulletColor = isRunning
          ? 'text-amber-500'
          : entry.status === 'failed'
            ? 'text-red-500'
            : 'text-emerald-500';
        const taskPreview = entry.task.length > 60 ? entry.task.slice(0, 57) + '…' : entry.task;

        return (
          <div key={entry.subagentId} className="mb-0.5">
            <div className="flex items-start gap-0">
              <span className="shrink-0">&nbsp;&nbsp;</span>
              <span className={cn('text-[11px] mr-0.5', bulletColor, isRunning && 'animate-pulse')}>⏺</span>
              <span className="text-cyan-600 dark:text-cyan-400 font-semibold">{entry.agentName || 'Task'}</span>
              <span className="text-slate-700 dark:text-slate-200">({taskPreview})</span>
            </div>

            {isRunning ? (
              entry.toolSteps && entry.toolSteps.length > 0 ? (<>
                {entry.toolSteps.slice(-3).map((step, si, arr) => {
                  const isLastStep = si === arr.length - 1;
                  const connector = isLastStep ? '⎿' : '│';
                  const stepColor = step.status === 'done' ? 'text-emerald-500' : step.status === 'failed' ? 'text-red-500' : 'text-amber-500';
                  return (
                    <div key={si} className="flex items-start gap-0 text-[11px] pl-4">
                      <span className="text-slate-400 dark:text-slate-600 select-none shrink-0">{connector}&nbsp;&nbsp;</span>
                      <span className={cn('mr-0.5', stepColor)}>⏺</span>
                      <span className={cn('font-medium', step.status === 'failed' ? 'text-red-600 dark:text-red-400' : 'text-cyan-600 dark:text-cyan-400')}>{step.toolName}</span>
                      {step.args && (
                        <span className="text-slate-400 dark:text-slate-500">({truncateDetail(step.args, 50)})</span>
                      )}
                    </div>
                  );
                })}
                {entry.toolSteps.length > 3 && (
                  <div className="text-[11px] pl-4 text-slate-400 dark:text-slate-500 italic">+{entry.toolSteps.length - 3} more</div>
                )}
              </>) : entry.currentActivity ? (
                <div className="flex items-start gap-0 text-[11px] pl-4 text-slate-400 dark:text-slate-500">
                  <span className="select-none shrink-0">⎿&nbsp;&nbsp;</span>
                  <span>{entry.currentActivity}</span>
                </div>
              ) : null
            ) : showExpanded && entry.toolSteps && entry.toolSteps.length > 0 ? (
              entry.toolSteps.map((step, si) => {
                const isLastStep = si === entry.toolSteps.length - 1;
                const connector = isLastStep ? '⎿' : '│';
                const stepBulletColor = step.status === 'done' ? 'text-emerald-500' : step.status === 'failed' ? 'text-red-500' : 'text-amber-500';
                return (
                  <div key={si} className="flex items-start gap-0 text-[11px] pl-4">
                    <span className="text-slate-400 dark:text-slate-600 select-none shrink-0">{connector}&nbsp;&nbsp;</span>
                    <span className={cn('mr-0.5', stepBulletColor)}>⏺</span>
                    <span className={cn('font-medium', step.status === 'failed' ? 'text-red-600 dark:text-red-400' : 'text-cyan-600 dark:text-cyan-400')}>{step.toolName}</span>
                    {step.args && (
                      <span className="text-slate-400 dark:text-slate-500">({truncateDetail(step.args, 60)})</span>
                    )}
                  </div>
                );
              })
            ) : !isRunning ? (
              <div className="flex items-start gap-0 text-[11px] pl-4">
                <span className="text-slate-400 dark:text-slate-600 select-none shrink-0">⎿&nbsp;&nbsp;</span>
                <span className="text-slate-400 dark:text-slate-500 italic">
                  Done ({[
                    `${entry.toolCount} tool use${entry.toolCount === 1 ? '' : 's'}`,
                    ...(entry.contextTokens > 0 ? [`${formatCompactTokens(entry.contextTokens)} tokens`] : []),
                  ].join(' \u00b7 ')})
                </span>
              </div>
            ) : null}
          </div>
        );
      })}

      {allDone && !showExpanded && !isGenerating && (
        <div className="text-[11px] text-slate-400 dark:text-slate-500 pl-4 italic">(click to expand)</div>
      )}
    </div>
  );
};
