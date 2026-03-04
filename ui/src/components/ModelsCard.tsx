import React from 'react';
import { Bot, Star } from 'lucide-react';
import { cn } from '../lib/cn';
import type { AgentInfo, ChatMessage, ModelInfo, OllamaPsResponse } from '../types';

const formatCompactInt = (n: number) => {
  if (!Number.isFinite(n)) return '';
  if (n >= 1_000_000) return `${Math.round(n / 100_000) / 10}m`;
  if (n >= 10_000) return `${Math.round(n / 1000)}k`;
  if (n >= 1_000) return `${Math.round(n / 100) / 10}k`;
  return `${n}`;
};

export const ModelsCard: React.FC<{
  models: ModelInfo[];
  agents: AgentInfo[];
  ollamaStatus: OllamaPsResponse | null;
  chatMessages: ChatMessage[];
  tokensPerSec?: number;
  activeModelId?: string;
  agentContext?: Record<string, { tokens: number; messages: number; tokenLimit?: number }>;
  defaultModels?: string[];
  onToggleDefault?: (modelId: string) => void;
}> = ({ models, agents, ollamaStatus, tokensPerSec, activeModelId, agentContext, defaultModels = [], onToggleDefault }) => {
  const tps = Number.isFinite(Number(tokensPerSec)) ? Number(tokensPerSec) : 0;

  return (
    <div className="px-3 py-2 space-y-2">
      {models.map((m) => {
        const isActive = ollamaStatus?.models.some((om) => om.name.includes(m.model) || m.model.includes(om.name));
        const activeInfo = ollamaStatus?.models.find((om) => om.name.includes(m.model) || m.model.includes(om.name));
        const assignedAgents = agents.filter((a) => {
          const agentModel = a.model?.toLowerCase();
          const modelId = m.id.toLowerCase();
          const modelName = m.model.toLowerCase();
          return agentModel === modelId || agentModel === modelName || (!agentModel && models.length === 1);
        });

        const isStarred = defaultModels.includes(m.id);
        // Show token rate on the model currently generating, not just any Ollama model
        const isGeneratingModel = activeModelId
          ? (m.id.toLowerCase() === activeModelId.toLowerCase() || m.model.toLowerCase() === activeModelId.toLowerCase())
          : false;
        const showTps = tps > 0 && isGeneratingModel;
        return (
          <div key={m.id} className={cn(
            'rounded-lg border bg-slate-50/50 dark:bg-white/[0.02] p-2.5 space-y-1.5',
            isStarred ? 'border-amber-300/60 dark:border-amber-700/40' : 'border-slate-100 dark:border-white/5'
          )}>
            {/* Model name + status */}
            <div className="flex items-center justify-between gap-1">
              <div className="flex items-center gap-1.5 min-w-0">
                <div className={cn('w-1.5 h-1.5 rounded-full shrink-0', (isActive || isGeneratingModel) ? 'bg-green-500 animate-pulse' : 'bg-slate-300 dark:bg-slate-600')} />
                <span className="font-mono font-bold text-[11px] truncate">{m.model || m.id}</span>
              </div>
              <div className="flex items-center gap-1.5 shrink-0">
                <span className="text-[8px] text-slate-400 uppercase tracking-wide">{m.provider}</span>
                {onToggleDefault && (
                  <button
                    onClick={() => onToggleDefault(m.id)}
                    className={cn(
                      'p-0.5 transition-colors',
                      isStarred ? 'text-amber-500 hover:text-amber-600' : 'text-slate-300 hover:text-amber-500'
                    )}
                    title={isStarred ? 'Remove from defaults' : 'Add to defaults'}
                  >
                    <Star size={11} fill={isStarred ? 'currentColor' : 'none'} />
                  </button>
                )}
              </div>
            </div>

            {/* Model stats when active */}
            {(activeInfo || showTps) && (
              <div className="flex items-center gap-3 text-[9px] text-slate-400 font-mono">
                {activeInfo && <span>{activeInfo.details.parameter_size}</span>}
                {activeInfo && <span>{activeInfo.details.quantization_level}</span>}
                {activeInfo && <span>{(activeInfo.size_vram / 1024 / 1024 / 1024).toFixed(1)}GB</span>}
                {showTps && <span className="text-emerald-500 font-semibold">{tps.toFixed(1)} tok/s</span>}
              </div>
            )}

            {/* Assigned agents with context */}
            {assignedAgents.length > 0 && (
              <div className="space-y-1 pt-1 border-t border-slate-100 dark:border-white/5">
                {assignedAgents.map((agent) => {
                  const ctx = agentContext?.[agent.name.toLowerCase()];
                  const tokens = ctx?.tokens || 0;
                  const limit = ctx?.tokenLimit && ctx.tokenLimit > 0 ? ctx.tokenLimit : null;
                  const pct = limit ? Math.round((tokens / limit) * 100) : null;

                  return (
                    <div key={agent.name} className="flex items-center gap-1.5 text-[10px]">
                      <Bot size={9} className="text-purple-400 shrink-0" />
                      <span className="font-semibold uppercase text-[9px] text-slate-500 dark:text-slate-400 w-16 truncate">{agent.name}</span>
                      {ctx ? (
                        <>
                          <div className="flex-1 h-1 rounded-full bg-slate-200 dark:bg-white/10 overflow-hidden">
                            <div
                              className={cn(
                                'h-full rounded-full transition-all',
                                (pct ?? 0) > 80 ? 'bg-red-400' : (pct ?? 0) > 50 ? 'bg-amber-400' : 'bg-blue-400'
                              )}
                              style={{ width: `${Math.min(pct ?? 0, 100)}%` }}
                            />
                          </div>
                          <span className="text-[8px] text-slate-400 tabular-nums shrink-0">
                            {formatCompactInt(tokens)}{limit ? `/${formatCompactInt(limit)}` : ''}
                          </span>
                        </>
                      ) : (
                        <span className="text-[8px] text-slate-300 dark:text-slate-600 italic">no context</span>
                      )}
                    </div>
                  );
                })}
              </div>
            )}
          </div>
        );
      })}
    </div>
  );
};
