import React from 'react';
import { Brain, Star } from 'lucide-react';
import { cn } from '../lib/cn';
import type { AgentInfo, ChatMessage, ModelInfo, OllamaPsResponse } from '../types';

/** Check if a model supports reasoning effort control. */
function supportsReasoningEffort(model: ModelInfo): boolean {
  const m = (model.model || '').toLowerCase();
  const p = (model.provider || '').toLowerCase();
  // OpenAI reasoning models
  if (m.includes('gpt-5') || m.includes('o3') || m.includes('o4') || m.includes('o1')) return true;
  // Gemini 2.5 thinking models
  if (m.includes('gemini') && m.includes('2.5')) return true;
  // Claude models with extended thinking
  if (p === 'anthropic' || m.includes('claude')) return true;
  // ChatGPT provider (likely GPT-5/o-series)
  if (p === 'chatgpt') return true;
  // DeepSeek reasoning models
  if (m.includes('deepseek-r') || m.includes('deepseek-reasoner')) return true;
  return false;
}

export const ModelsCard: React.FC<{
  models: ModelInfo[];
  agents: AgentInfo[];
  ollamaStatus: OllamaPsResponse | null;
  chatMessages: ChatMessage[];
  activeModelId?: string;
  defaultModels?: string[];
  onToggleDefault?: (modelId: string) => void;
  onChangeReasoningEffort?: (modelId: string, effort: string | null) => void;
}> = ({ models, agents, ollamaStatus, activeModelId, defaultModels = [], onToggleDefault, onChangeReasoningEffort }) => {
  const scrollContainerRef = React.useRef<HTMLDivElement>(null);

  // Scroll to the default (starred) model on mount
  React.useEffect(() => {
    if (!scrollContainerRef.current || defaultModels.length === 0) return;
    const el = scrollContainerRef.current.querySelector('[data-default-model]');
    if (el) el.scrollIntoView({ block: 'center', behavior: 'smooth' });
  }, []);

  return (
    <div ref={scrollContainerRef} className="px-3 py-2 space-y-2 max-h-48 overflow-y-auto">
      {models.map((m) => {
        const isActive = ollamaStatus?.models?.some((om) => om.name.includes(m.model) || m.model.includes(om.name));
        const activeInfo = ollamaStatus?.models?.find((om) => om.name.includes(m.model) || m.model.includes(om.name));
        const isStarred = defaultModels.includes(m.id);
        // Show token rate on the model currently generating, not just any Ollama model
        const isGeneratingModel = activeModelId
          ? (m.id.toLowerCase() === activeModelId.toLowerCase() || m.model.toLowerCase() === activeModelId.toLowerCase())
          : false;
        return (
          <div key={m.id} {...(isStarred ? { 'data-default-model': '' } : {})} className={cn(
            'rounded-lg border bg-slate-50/50 dark:bg-white/[0.02] p-2.5 space-y-1.5',
            isStarred ? 'border-amber-300/60 dark:border-amber-700/40' : 'border-slate-100 dark:border-white/5'
          )}>
            {/* Model name + status */}
            <div className="flex items-center justify-between gap-1">
              <div className="flex items-center gap-1.5 min-w-0">
                <div className={cn('w-1.5 h-1.5 rounded-full shrink-0', (isActive || isGeneratingModel) ? 'bg-green-500 animate-pulse' : 'bg-slate-300 dark:bg-slate-600')} />
                <span className="font-mono font-bold text-[12px] truncate">{m.model || m.id}</span>
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

            {/* Reasoning effort switcher — only for models that support it */}
            {onChangeReasoningEffort && supportsReasoningEffort(m) && (() => {
              // Effective effort: explicit setting, or 'medium' as default
              const effective = m.reasoning_effort || 'medium';
              const isDefault = !m.reasoning_effort;
              return (
                <div className="flex items-center gap-1.5 text-[10px]">
                  <Brain size={9} className="text-purple-400 shrink-0" />
                  <span className="text-slate-400">Reasoning:</span>
                  {(['low', 'medium', 'high'] as const).map((level) => {
                    const isActive = effective === level;
                    const isDefaultMedium = isDefault && level === 'medium';
                    return (
                      <button
                        key={level}
                        onClick={() => onChangeReasoningEffort(m.id, level === 'medium' ? null : level)}
                        className={cn(
                          'px-1.5 py-0.5 rounded text-[8px] font-semibold uppercase transition-all',
                          isActive
                            ? level === 'high' ? 'bg-purple-500/20 text-purple-400 border border-purple-500/30'
                              : level === 'medium' ? 'bg-blue-500/20 text-blue-400 border border-blue-500/30'
                              : 'bg-slate-500/20 text-slate-400 border border-slate-500/30'
                            : 'text-slate-500 hover:text-slate-300 border border-transparent hover:border-slate-600'
                        )}
                        title={isDefaultMedium ? 'Default' : ''}
                      >
                        {level === 'low' ? 'Lo' : level === 'medium' ? 'Med' : 'Hi'}
                      </button>
                    );
                  })}
                </div>
              );
            })()}

            {/* Ollama model stats (param size, quant, vram) */}
            {activeInfo && (
              <div className="flex items-center gap-3 text-[10px] text-slate-400 font-mono">
                <span>{activeInfo.details.parameter_size}</span>
                <span>{activeInfo.details.quantization_level}</span>
                <span>{(activeInfo.size_vram / 1024 / 1024 / 1024).toFixed(1)}GB</span>
              </div>
            )}
          </div>
        );
      })}
    </div>
  );
};
