import React from 'react';
import { Bot, Cpu } from 'lucide-react';
import { cn } from '../lib/cn';
import type { AgentInfo, AgentRunSummary, AgentWorkInfo, WorkspaceState } from '../types';

const formatTime = (ts?: number | null) => {
  if (!ts || ts <= 0) return '-';
  return new Date(ts * 1000).toLocaleTimeString();
};

const contextPercent = (tokens: number, tokenLimit?: number) => {
  const limit = tokenLimit && tokenLimit > 0 ? tokenLimit : undefined;
  if (!limit) return null;
  if (!tokens || tokens <= 0) return 0;
  return Math.round((tokens / limit) * 100);
};

const formatCompactInt = (n: number) => {
  if (!Number.isFinite(n)) return '';
  if (n >= 1_000_000) return `${Math.round(n / 100_000) / 10}m`;
  if (n >= 10_000) return `${Math.round(n / 1000)}k`;
  if (n >= 1_000) return `${Math.round(n / 100) / 10}k`;
  return `${n}`;
};

const statusStyle = (status: string) => {
  switch (status) {
    case 'calling_tool':
      return 'bg-amber-500/20 text-amber-600 dark:text-amber-400 animate-pulse';
    case 'model_loading':
      return 'bg-indigo-500/20 text-indigo-600 dark:text-indigo-400 animate-pulse';
    case 'thinking':
      return 'bg-blue-500/20 text-blue-500 animate-pulse';
    case 'working':
      return 'bg-green-500/20 text-green-500 animate-pulse';
    default:
      return 'bg-slate-200/60 dark:bg-white/5 text-slate-400 dark:text-slate-500';
  }
};

const statusLabel = (status: string, text?: string) => {
  if (text) return text;
  switch (status) {
    case 'calling_tool': return 'Tool';
    case 'model_loading': return 'Loading';
    case 'thinking': return 'Thinking';
    case 'working': return 'Working';
    default: return 'Idle';
  }
};

export const AgentsCard: React.FC<{
  agents: AgentInfo[];
  workspaceState: WorkspaceState | null;
  isRunning: boolean;
  selectedAgent: string;
  setSelectedAgent: (value: string) => void;
  agentStatus?: Record<string, 'idle' | 'model_loading' | 'thinking' | 'calling_tool' | 'working'>;
  agentStatusText?: Record<string, string>;
  agentWork?: Record<string, AgentWorkInfo>;
  agentRunSummary?: Record<string, AgentRunSummary>;
  agentContext?: Record<string, { tokens: number; messages: number; tokenLimit?: number }>;
  projectRoot?: string;
}> = ({ agents, workspaceState, isRunning, selectedAgent, setSelectedAgent, agentStatus, agentStatusText, agentWork, agentRunSummary, agentContext }) => {
  return (
    <div className="px-3 py-2 space-y-1.5">
      {agents.map((agent) => {
        const id = agent.name.toLowerCase();
        const isSelected = id === selectedAgent.toLowerCase();
        const status = agentStatus?.[agent.name] ?? ((isRunning && selectedAgent === id) ? 'thinking' : 'idle');
        const work = agentWork?.[agent.name];
        const run = agentRunSummary?.[id];
        const context = agentContext?.[id];
        const contextTokens = Number(context?.tokens || 0);
        const contextLimit =
          context && typeof context.tokenLimit === 'number' && context.tokenLimit > 0
            ? Number(context.tokenLimit)
            : null;
        const contextPct = contextPercent(contextTokens, contextLimit ?? undefined);
        const isActive = status !== 'idle';

        return (
          <button
            type="button"
            key={agent.name}
            onClick={() => setSelectedAgent(id)}
            className={cn(
              'w-full text-left px-2.5 py-2 rounded-lg border text-[11px] transition-colors',
              isSelected
                ? 'bg-blue-50 dark:bg-blue-500/10 border-blue-300 dark:border-blue-500/30 ring-1 ring-blue-400/30'
                : isActive
                  ? 'bg-blue-50/30 dark:bg-blue-500/5 border-blue-200 dark:border-blue-500/15 hover:border-blue-300 dark:hover:border-blue-500/25'
                  : 'bg-slate-50/50 dark:bg-white/[0.02] border-slate-100 dark:border-white/5 hover:border-slate-200 dark:hover:border-white/10'
            )}
          >
            {/* Row 1: Name + Status + Model */}
            <div className="flex items-center gap-1.5 min-w-0">
              <Bot size={12} className={cn('shrink-0', isSelected ? 'text-blue-500' : 'text-purple-500')} />
              <span className="font-bold uppercase tracking-tight text-[12px]">{agent.name}</span>
              <span className={cn('text-[8px] font-bold px-1.5 py-px rounded-full uppercase tracking-wide shrink-0', statusStyle(status))}>
                {statusLabel(status, agentStatusText?.[agent.name])}
              </span>
              {agent.model && (
                <span className="ml-auto flex items-center gap-0.5 text-[8px] text-slate-400 dark:text-slate-500 font-mono shrink-0" title={`Model: ${agent.model}`}>
                  <Cpu size={8} />
                  {agent.model.length > 16 ? agent.model.slice(0, 16) + '..' : agent.model}
                </span>
              )}
            </div>

            {/* Row 2: Context bar */}
            {context ? (
              <div className="mt-1.5 flex items-center gap-1.5">
                <span className="text-[8px] text-slate-400 w-7 shrink-0">CTX</span>
                <div className="flex-1 h-1.5 rounded-full bg-slate-200 dark:bg-white/10 overflow-hidden">
                  <div
                    className={cn(
                      'h-full rounded-full transition-all',
                      (contextPct ?? 0) > 80 ? 'bg-red-400' : (contextPct ?? 0) > 50 ? 'bg-amber-400' : 'bg-blue-400'
                    )}
                    style={{ width: `${Math.min(contextPct ?? 0, 100)}%` }}
                  />
                </div>
                <span className="text-[8px] text-slate-400 dark:text-slate-500 tabular-nums shrink-0">
                  {formatCompactInt(contextTokens)}{contextLimit ? ` / ${formatCompactInt(contextLimit)}` : ''}
                  {contextPct !== null ? ` (${contextPct}%)` : ''}
                </span>
              </div>
            ) : (
              <div className="mt-1 text-[8px] text-slate-300 dark:text-slate-600 italic">no context yet</div>
            )}

            {/* Row 3: Work info (only when active) */}
            {isActive && work?.file && (
              <div className="mt-1 text-slate-500 dark:text-slate-400 truncate">
                {work.file}
                {work.activeCount > 1 ? ` (+${work.activeCount - 1})` : ''}
              </div>
            )}

            {/* Row 4: Run badges — hide stale "running" when agent is actually idle */}
            {run && !(run.status === 'running' && !isActive) && (
              <div className="mt-1 flex items-center gap-1 flex-wrap">
                <span
                  className={cn(
                    'text-[8px] px-1 py-px rounded-full uppercase font-semibold',
                    run.status === 'running'
                      ? 'bg-green-500/20 text-green-600 dark:text-green-400'
                      : run.status === 'failed'
                        ? 'bg-red-500/20 text-red-600 dark:text-red-400'
                        : run.status === 'cancelled'
                          ? 'bg-amber-500/20 text-amber-600 dark:text-amber-400'
                          : 'bg-slate-500/10 text-slate-500'
                  )}
                >
                  {run.status}
                </span>
                {run.child_count > 0 && (
                  <span className="text-[8px] px-1 py-px rounded-full bg-purple-500/10 text-purple-500 font-semibold">
                    {run.child_count} sub
                  </span>
                )}
                <span className="text-[8px] text-slate-400 ml-auto tabular-nums">
                  {formatTime(run.last_event_at)}
                </span>
              </div>
            )}
          </button>
        );
      })}

      {/* Active Task */}
      {workspaceState?.active_task && (
        <div className="pt-1.5 mt-1 border-t border-slate-100 dark:border-white/5">
          <div className="text-[10px] font-bold uppercase tracking-widest text-slate-400 mb-1">Active Task</div>
          <p className="text-[11px] text-slate-500 dark:text-slate-400 italic truncate">
            {workspaceState.active_task[1].substring(0, 100)}
          </p>
        </div>
      )}
    </div>
  );
};
