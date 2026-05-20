import React from 'react';
import { cn } from '../../lib/cn';
import type { SubagentTreeEntry } from '../../types';
import { formatCompactTokens } from './utils/activity';
import { truncateDetail } from './utils/content-block';
import { useInteractionStore } from '../../stores/interactionStore';

/** Subagent tree view — Claude Code-style per-entry Task() blocks.
 *
 * `compact` mode strips the live "currentActivity" / toolSteps body
 * and shows only the one-line status row per entry. Used in the
 * parent's main-chat bubble while the SubagentPane on the right
 * carries the full detail surface. Defaults to false so the pane
 * keeps its existing rich view.
 */
export const SubagentTreeView: React.FC<{
  entries: SubagentTreeEntry[];
  isGenerating: boolean;
  isExpanded: boolean;
  onToggle: () => void;
  compact?: boolean;
}> = ({ entries, isGenerating, isExpanded, onToggle, compact = false }) => {
  const allDone = entries.every((e) => e.status !== 'running');

  const showExpanded = isGenerating || isExpanded;

  // Subagent needs the user → swap the status glyph to ❓ so the inline
  // line in the parent bubble doesn't look "running normally" while a
  // widget is waiting in the SubagentPane. Reads the same single
  // pendingAskUser slot the routing predicate uses.
  const pendingAskUser = useInteractionStore((s) => s.pendingAskUser);
  const needsUserAgentId = pendingAskUser?.agentId ?? null;
  const entryNeedsUser = (entry: SubagentTreeEntry): boolean =>
    !!needsUserAgentId &&
    (entry.subagentId === needsUserAgentId || entry.agentName === needsUserAgentId);

  return (
    <div
      className={cn('mb-1.5 font-mono text-[12px]', !isGenerating && allDone && 'cursor-pointer select-none')}
      onClick={!isGenerating && allDone ? onToggle : undefined}
    >
      {entries.map((entry, entryIdx) => {
        const isRunning = entry.status === 'running';
        const needsUser = entryNeedsUser(entry);
        const bulletColor = needsUser
          ? 'text-blue-500'
          : isRunning
            ? 'text-amber-500'
            : entry.status === 'failed'
              ? 'text-red-500'
              : 'text-emerald-500';
        const bulletGlyph = needsUser ? '❓' : '⏺';
        const statusSuffix = needsUser
          ? ' — needs your input ↗'
          : isRunning
            ? ' — running…'
            : entry.status === 'failed'
              ? ' — failed'
              : ` — done (${entry.toolCount} tool use${entry.toolCount === 1 ? '' : 's'}${entry.contextTokens > 0 ? ` · ${formatCompactTokens(entry.contextTokens)} tokens` : ''})`;
        const statusColor = needsUser
          ? 'text-blue-600 dark:text-blue-400'
          : isRunning
            ? 'text-amber-600 dark:text-amber-400'
            : entry.status === 'failed'
              ? 'text-red-600 dark:text-red-400'
              : 'text-emerald-600 dark:text-emerald-400';

        return (
          <div key={`${entry.subagentId}-${entryIdx}`} className="mb-0.5">
            <div className="flex items-start gap-0">
              <span className="shrink-0">&nbsp;&nbsp;</span>
              <span className={cn('text-[11px] mr-0.5', bulletColor, (isRunning || needsUser) && 'animate-pulse')}>{bulletGlyph}</span>
              <span className="text-cyan-600 dark:text-cyan-400 font-semibold">{entry.subagentId || entry.agentName || 'Task'}</span>
              <span className={cn('text-[11px] ml-1', statusColor)}>{statusSuffix}</span>
            </div>

            {compact ? null : isRunning ? (
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
              <>
                {entry.toolSteps.map((step, si) => {
                  const isLastStep = si === entry.toolSteps.length - 1 && !entry.resultText;
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
                })}
                {entry.resultText && (
                  <div className="flex items-start gap-0 text-[11px] pl-4 text-slate-500 dark:text-slate-400">
                    <span className="text-slate-400 dark:text-slate-600 select-none shrink-0">⎿&nbsp;&nbsp;</span>
                    <span className="whitespace-pre-wrap break-words">{entry.resultText}</span>
                  </div>
                )}
              </>
            ) : showExpanded && entry.resultText ? (
              <div className="flex items-start gap-0 text-[11px] pl-4 text-slate-500 dark:text-slate-400">
                <span className="text-slate-400 dark:text-slate-600 select-none shrink-0">⎿&nbsp;&nbsp;</span>
                <span className="whitespace-pre-wrap break-words">{entry.resultText}</span>
              </div>
            ) : null}
          </div>
        );
      })}

      {!compact && allDone && !showExpanded && !isGenerating && (
        <div className="text-[11px] text-slate-400 dark:text-slate-500 pl-4 italic">(click to expand)</div>
      )}
    </div>
  );
};
