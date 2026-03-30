import React, { useCallback, useEffect, useState } from 'react';
import { Eye, EyeOff, LogIn, LogOut, Plus, Star, Trash2 } from 'lucide-react';
import type { AppConfig, ModelConfigUI, ModelHealthInfo, OllamaPsResponse } from '../types';

const inputCls = 'w-full bg-slate-100 dark:bg-white/5 border border-slate-200 dark:border-white/10 rounded-lg px-3 py-2 text-xs outline-none focus:ring-1 focus:ring-blue-500/50';
const labelCls = 'text-[11px] font-bold uppercase tracking-wider text-slate-500 dark:text-slate-400';
const sectionCls = 'bg-white dark:bg-[#141414] rounded-xl border border-slate-200 dark:border-white/5 shadow-sm p-5';

const PROVIDER_PRESETS: Record<string, { url: string; defaultModel: string; placeholder: string; authMode?: string }> = {
  ollama: { url: 'http://127.0.0.1:11434', defaultModel: '', placeholder: 'e.g. qwen3:32b' },
  chatgpt: { url: 'https://chatgpt.com/backend-api/codex', defaultModel: 'o3', placeholder: 'e.g. gpt-5.4, o3, gpt-4o', authMode: 'chatgpt_oauth' },
  openai: { url: 'https://api.openai.com/v1', defaultModel: 'gpt-4o', placeholder: 'e.g. gpt-4o' },
  gemini: { url: 'https://generativelanguage.googleapis.com/v1beta/openai', defaultModel: 'gemini-2.5-flash', placeholder: 'e.g. gemini-2.5-flash, gemini-3-flash-preview' },
  groq: { url: 'https://api.groq.com/openai/v1', defaultModel: 'llama-3.3-70b-versatile', placeholder: 'e.g. llama-3.3-70b-versatile' },
  deepseek: { url: 'https://api.deepseek.com/v1', defaultModel: 'deepseek-chat', placeholder: 'e.g. deepseek-chat' },
  openrouter: { url: 'https://openrouter.ai/api/v1', defaultModel: '', placeholder: 'e.g. google/gemini-2.5-pro' },
  github: { url: 'https://models.inference.ai.azure.com', defaultModel: 'gpt-4o-mini', placeholder: 'e.g. gpt-4o-mini' },
};

const emptyModel = (): ModelConfigUI => ({
  id: '',
  provider: 'ollama',
  url: 'http://127.0.0.1:11434',
  model: '',
  api_key: null,
  keep_alive: null,
  tags: [],
});

type CredentialsMap = Record<string, { api_key?: string | null }>;

const HealthDot: React.FC<{ health: ModelHealthInfo | undefined; ollamaStatus: 'connected' | 'disconnected' | 'na' }> = ({ health, ollamaStatus }) => {
  // Priority: health tracker status > Ollama ps status
  if (health && health.health !== 'healthy') {
    const cls = health.health === 'quota_exhausted' ? 'bg-amber-500' : 'bg-red-500';
    const label = health.health === 'quota_exhausted'
      ? `Quota exhausted${health.since_secs ? ` (${Math.round(health.since_secs / 60)}m ago)` : ''}`
      : `Down${health.last_error ? `: ${health.last_error.slice(0, 60)}` : ''}`;
    return (
      <span className="inline-flex items-center gap-1.5" title={label}>
        <span className={`w-2 h-2 rounded-full ${cls}`} />
        <span className="text-[11px] text-slate-500">
          {health.health === 'quota_exhausted' ? 'Quota' : 'Down'}
        </span>
      </span>
    );
  }

  const cls = ollamaStatus === 'connected' ? 'bg-green-500'
    : ollamaStatus === 'disconnected' ? 'bg-red-500' : 'bg-slate-400';
  const label = ollamaStatus === 'connected' ? 'Healthy'
    : ollamaStatus === 'disconnected' ? 'Disconnected' : 'N/A';
  return (
    <span className="inline-flex items-center gap-1.5" title={label}>
      <span className={`w-2 h-2 rounded-full ${cls}`} />
      <span className="text-[11px] text-slate-500">{label}</span>
    </span>
  );
};

export const ModelsTab: React.FC<{
  config: AppConfig;
  onChange: (config: AppConfig) => void;
  onCredsDirtyChange?: (dirty: boolean) => void;
  saveCredsRef?: React.MutableRefObject<(() => Promise<void>) | null>;
}> = ({ config, onChange, onCredsDirtyChange, saveCredsRef }) => {
  const [ollamaStatus, setOllamaStatus] = useState<OllamaPsResponse | null>(null);
  const [healthMap, setHealthMap] = useState<Record<string, ModelHealthInfo>>({});
  const [credentials, setCredentials] = useState<CredentialsMap>({});
  const [localKeys, setLocalKeys] = useState<Record<string, string>>({});
  const [revealKeys, setRevealKeys] = useState<Record<string, boolean>>({});
  const [credsDirty, setCredsDirty] = useState(false);
  const [codexAuthStatus, setCodexAuthStatus] = useState<{ authenticated: boolean; account_id?: string } | null>(null);
  const [codexAuthLoading, setCodexAuthLoading] = useState(false);

  const defaultModels = config.routing?.default_models ?? [];

  const hasOllamaModels = config.models?.some((m: { provider: string }) => m.provider === 'ollama') ?? false;

  const fetchOllamaStatus = useCallback(async () => {
    if (!hasOllamaModels) return;
    try {
      const resp = await fetch('/api/utils/ollama-status');
      if (resp.ok) setOllamaStatus(await resp.json());
      else setOllamaStatus(null);
    } catch { setOllamaStatus(null); }
  }, [hasOllamaModels]);

  const fetchHealth = useCallback(async () => {
    try {
      const resp = await fetch('/api/models/health');
      if (resp.ok) {
        const data: ModelHealthInfo[] = await resp.json();
        const map: Record<string, ModelHealthInfo> = {};
        for (const h of data) map[h.id] = h;
        setHealthMap(map);
      }
    } catch { /* ignore */ }
  }, []);

  const fetchCredentials = useCallback(async () => {
    try {
      const resp = await fetch('/api/credentials');
      if (resp.ok) {
        const data: CredentialsMap = await resp.json();
        setCredentials(data);
        const keys: Record<string, string> = {};
        for (const [id, entry] of Object.entries(data)) {
          keys[id] = entry.api_key || '';
        }
        setLocalKeys(keys);
      }
    } catch { /* ignore */ }
  }, []);

  const fetchCodexAuthStatus = useCallback(async () => {
    try {
      const resp = await fetch('/api/auth/codex/status');
      if (resp.ok) setCodexAuthStatus(await resp.json());
    } catch { /* ignore */ }
  }, []);

  const handleCodexLogin = async () => {
    setCodexAuthLoading(true);
    try {
      await fetch('/api/auth/codex/login', { method: 'POST' });
      // Poll for auth status (browser flow takes time)
      const poll = setInterval(async () => {
        const resp = await fetch('/api/auth/codex/status');
        if (resp.ok) {
          const data = await resp.json();
          setCodexAuthStatus(data);
          if (data.authenticated) {
            clearInterval(poll);
            setCodexAuthLoading(false);
          }
        }
      }, 2000);
      // Timeout after 5 min
      setTimeout(() => { clearInterval(poll); setCodexAuthLoading(false); }, 300_000);
    } catch {
      setCodexAuthLoading(false);
    }
  };

  const handleCodexLogout = async () => {
    await fetch('/api/auth/codex/logout', { method: 'POST' });
    setCodexAuthStatus({ authenticated: false });
  };

  useEffect(() => {
    fetchOllamaStatus();
    fetchHealth();
    fetchCredentials();
    fetchCodexAuthStatus();
    const timer = setInterval(() => { fetchOllamaStatus(); fetchHealth(); }, 5000);
    return () => clearInterval(timer);
  }, [fetchOllamaStatus, fetchHealth, fetchCredentials, fetchCodexAuthStatus]);

  const updateModel = (index: number, field: keyof ModelConfigUI, value: string | null) => {
    const models = [...config.models];
    const oldId = models[index]?.id;
    const updated = { ...models[index], [field]: value };
    if (field === 'provider' && value && PROVIDER_PRESETS[value]) {
      updated.url = PROVIDER_PRESETS[value].url;
      // Auto-fill model name if currently empty
      if (!updated.model && PROVIDER_PRESETS[value].defaultModel) {
        updated.model = PROVIDER_PRESETS[value].defaultModel;
      }
      // Set auth_mode for ChatGPT OAuth provider
      updated.auth_mode = PROVIDER_PRESETS[value].authMode || null;
    }
    models[index] = updated;
    // When model ID is renamed, update default_models to keep it in sync
    let routing = config.routing;
    if (field === 'id' && oldId && value && oldId !== value && defaultModels.includes(oldId)) {
      const newDefaults = defaultModels.map((id) => id === oldId ? value : id);
      routing = { ...routing, default_models: newDefaults };
    }
    onChange({ ...config, models, routing });
  };

  const addModel = () => {
    onChange({ ...config, models: [emptyModel(), ...config.models] });
  };

  const removeModel = (index: number) => {
    const modelId = config.models[index]?.id;
    const newModels = config.models.filter((_, i) => i !== index);
    // Also remove from default_models if present
    const newDefaults = defaultModels.filter((id) => id !== modelId);
    onChange({
      ...config,
      models: newModels,
      routing: { ...config.routing, default_models: newDefaults },
    });
  };

  const updateLocalKey = (modelId: string, value: string) => {
    setLocalKeys((prev) => ({ ...prev, [modelId]: value }));
    setCredsDirty(true);
    onCredsDirtyChange?.(true);
  };

  const saveCredentials = useCallback(async () => {
    const body: CredentialsMap = {};
    for (const model of config.models) {
      if (!model.id) continue;
      const val = localKeys[model.id];
      if (val !== undefined && val !== '***') {
        body[model.id] = { api_key: val || null };
      }
    }
    try {
      await fetch('/api/credentials', {
        method: 'PUT',
        headers: { 'Content-Type': 'application/json' },
        body: JSON.stringify(body),
      });
      setCredsDirty(false);
      onCredsDirtyChange?.(false);
      fetchCredentials();
    } catch { /* ignore */ }
  }, [config.models, localKeys, fetchCredentials, onCredsDirtyChange]);

  // Expose saveCredentials to parent via ref
  useEffect(() => {
    if (saveCredsRef) saveCredsRef.current = saveCredentials;
    return () => { if (saveCredsRef) saveCredsRef.current = null; };
  }, [saveCredsRef, saveCredentials]);

  const getOllamaStatus = (model: ModelConfigUI): 'connected' | 'disconnected' | 'na' => {
    if (model.provider !== 'ollama') return 'na';
    if (!ollamaStatus?.models) return 'disconnected';
    const running = ollamaStatus.models.some(
      (m) => m.name === model.model || m.model === model.model
    );
    return running ? 'connected' : 'disconnected';
  };

  const hasKey = (modelId: string) => !!(credentials[modelId]?.api_key);

  // Default model selection helpers
  const isDefault = (modelId: string) => defaultModels.includes(modelId);

  const toggleDefault = (modelId: string) => {
    // Single-select: set as default, or deselect if already the default
    const newDefaults = defaultModels.length === 1 && defaultModels[0] === modelId ? [] : [modelId];
    onChange({
      ...config,
      routing: { ...config.routing, default_models: newDefaults },
    });
  };


  const updateTags = (index: number, tagsStr: string) => {
    const tags = tagsStr.split(',').map((t) => t.trim()).filter(Boolean);
    const models = [...config.models];
    models[index] = { ...models[index], tags };
    onChange({ ...config, models });
  };

  return (
    <div className="space-y-6">
      {/* Default Model section */}
      {defaultModels.length > 0 && (
        <section className={sectionCls}>
          <h2 className="text-xs font-bold uppercase tracking-wider text-slate-700 dark:text-slate-300 mb-3">
            Default Model
          </h2>
          {(() => {
            const modelId = defaultModels[0];
            const model = config.models.find((m) => m.id === modelId);
            const health = healthMap[modelId];
            return (
              <div className="flex items-center gap-2 px-3 py-2 bg-slate-50 dark:bg-white/[0.02] rounded-lg border border-slate-100 dark:border-white/5">
                <span className="text-xs font-medium flex-1">
                  {modelId}
                  {model && model.model && <span className="text-[11px] text-slate-400 ml-1.5">({model.model})</span>}
                </span>
                <HealthDot health={health} ollamaStatus={model ? getOllamaStatus(model) : 'na'} />
                <button
                  onClick={() => toggleDefault(modelId)}
                  className="p-0.5 text-slate-400 hover:text-red-500"
                  title="Clear default"
                >
                  <Trash2 size={12} />
                </button>
              </div>
            );
          })()}
        </section>
      )}

      {/* Auto-fallback toggle */}
      <section className={sectionCls}>
        <label className="flex items-center justify-between cursor-pointer">
          <div>
            <span className="text-xs font-bold uppercase tracking-wider text-slate-700 dark:text-slate-300">Auto Fallback</span>
            <p className="text-[11px] text-slate-500 mt-0.5">Automatically try the next model on timeout, rate limit, or connection errors</p>
          </div>
          <input
            type="checkbox"
            checked={config.routing?.auto_fallback !== false}
            onChange={(e) => onChange({ ...config, routing: { ...config.routing, auto_fallback: e.target.checked } })}
            className="w-4 h-4 accent-blue-600"
          />
        </label>
      </section>

      {/* Model cards */}
      <section className={sectionCls}>
        <div className="flex items-center justify-between mb-4">
          <h2 className="text-xs font-bold uppercase tracking-wider text-slate-700 dark:text-slate-300">Models</h2>
          <button onClick={addModel} className="flex items-center gap-1 text-[11px] font-bold text-blue-600 hover:text-blue-700">
            <Plus size={12} /> Add Model
          </button>
        </div>
        <div className="space-y-4">
          {config.models.map((model, i) => {
            const health = healthMap[model.id];
            const modelIsDefault = model.id ? isDefault(model.id) : false;
            return (
              <div key={i} className={`bg-slate-50 dark:bg-white/[0.02] rounded-lg p-4 border relative ${modelIsDefault ? 'border-amber-300 dark:border-amber-700' : 'border-slate-100 dark:border-white/5'}`}>
                <div className="absolute top-3 right-3 flex items-center gap-3">
                  <HealthDot health={health} ollamaStatus={getOllamaStatus(model)} />
                  {model.id && (
                    <button
                      onClick={() => toggleDefault(model.id)}
                      className={`p-1 transition-colors ${modelIsDefault ? 'text-amber-500 hover:text-amber-600' : 'text-slate-300 hover:text-amber-500'}`}
                      title={modelIsDefault ? 'Remove from defaults' : 'Add to defaults'}
                    >
                      <Star size={14} fill={modelIsDefault ? 'currentColor' : 'none'} />
                    </button>
                  )}
                  <button
                    onClick={() => removeModel(i)}
                    className="p-1 text-slate-400 hover:text-red-500 transition-colors"
                    title="Remove model"
                  >
                    <Trash2 size={12} />
                  </button>
                </div>
                <div className="grid grid-cols-1 sm:grid-cols-2 gap-3">
                  <div>
                    <label className={labelCls}>ID</label>
                    <input className={inputCls} value={model.id} onChange={(e) => updateModel(i, 'id', e.target.value)} placeholder="e.g. local_ollama" />
                  </div>
                  <div>
                    <label className={labelCls}>Provider</label>
                    <select className={inputCls} value={model.provider} onChange={(e) => updateModel(i, 'provider', e.target.value)}>
                      <option value="ollama">Ollama (local)</option>
                      <option value="chatgpt">ChatGPT (Subscription)</option>
                      <option value="gemini">Google Gemini</option>
                      <option value="openai">OpenAI (API)</option>
                      <option value="groq">Groq</option>
                      <option value="deepseek">DeepSeek</option>
                      <option value="openrouter">OpenRouter</option>
                      <option value="github">GitHub Models</option>
                    </select>
                  </div>
                  <div>
                    <label className={labelCls}>URL</label>
                    <input className={inputCls} value={model.url} onChange={(e) => updateModel(i, 'url', e.target.value)} placeholder={PROVIDER_PRESETS[model.provider]?.url || 'http://...'} />
                  </div>
                  <div>
                    <label className={labelCls}>Model</label>
                    <input className={inputCls} value={model.model} onChange={(e) => updateModel(i, 'model', e.target.value)} placeholder={PROVIDER_PRESETS[model.provider]?.placeholder || 'e.g. model-name'} />
                  </div>
                  <div>
                    {model.auth_mode === 'chatgpt_oauth' ? (
                      <>
                        <label className={labelCls}>ChatGPT OAuth</label>
                        {codexAuthStatus?.authenticated ? (
                          <div className="flex items-center gap-2 mt-1">
                            <span className="inline-flex items-center gap-1.5 text-[11px] text-green-600 dark:text-green-400 font-medium">
                              <span className="w-2 h-2 rounded-full bg-green-500" />
                              Signed in
                              {codexAuthStatus.account_id && (
                                <span className="text-slate-400 font-normal ml-1">({codexAuthStatus.account_id.slice(0, 8)}…)</span>
                              )}
                            </span>
                            <button
                              onClick={handleCodexLogout}
                              className="flex items-center gap-1 text-[11px] text-red-500 hover:text-red-600 font-medium"
                            >
                              <LogOut size={10} /> Sign out
                            </button>
                          </div>
                        ) : (
                          <button
                            onClick={handleCodexLogin}
                            disabled={codexAuthLoading}
                            className="mt-1 flex items-center gap-1.5 px-3 py-1.5 text-[12px] font-semibold rounded-lg bg-green-600 text-white hover:bg-green-700 disabled:opacity-50 transition-colors"
                          >
                            <LogIn size={12} />
                            {codexAuthLoading ? 'Waiting for browser login...' : 'Sign in with ChatGPT'}
                          </button>
                        )}
                        <p className="text-[10px] text-slate-400 mt-1">Uses your ChatGPT Plus/Pro subscription — no API key needed.</p>
                      </>
                    ) : (
                      <>
                        <label className={labelCls}>
                          API Key
                          {hasKey(model.id) && <span className="ml-1 text-green-500 text-[8px]">(set)</span>}
                        </label>
                        <div className="relative">
                          <input
                            className={inputCls + ' pr-8'}
                            type={revealKeys[model.id] ? 'text' : 'password'}
                            value={localKeys[model.id] ?? ''}
                            onChange={(e) => updateLocalKey(model.id, e.target.value)}
                            placeholder={hasKey(model.id) ? '(stored in ~/.linggen/credentials.json)' : '(optional)'}
                          />
                          <button
                            type="button"
                            className="absolute right-2 top-1/2 -translate-y-1/2 text-slate-400 hover:text-slate-600"
                            onClick={() => setRevealKeys((prev) => ({ ...prev, [model.id]: !prev[model.id] }))}
                            tabIndex={-1}
                          >
                            {revealKeys[model.id] ? <EyeOff size={12} /> : <Eye size={12} />}
                          </button>
                        </div>
                      </>
                    )}
                  </div>
                  <div>
                    <label className={labelCls}>
                      Context Window
                      {!model.context_window && health?.context_window && (
                        <span className="font-normal text-slate-400 ml-1">
                          (auto: {health.context_window >= 1_000_000
                            ? `${(health.context_window / 1_000_000).toFixed(health.context_window % 1_000_000 === 0 ? 0 : 1)}M`
                            : `${Math.round(health.context_window / 1_000)}K`})
                        </span>
                      )}
                    </label>
                    <input
                      className={inputCls}
                      type="number"
                      value={model.context_window ?? ''}
                      onChange={(e) => updateModel(i, 'context_window', e.target.value ? Number(e.target.value) : null)}
                      placeholder={health?.context_window ? `${Math.round(health.context_window / 1_000)}K (auto-detected)` : 'e.g. 128000'}
                    />
                  </div>
                  <div>
                    <label className={labelCls}>Keep Alive</label>
                    <input className={inputCls} value={model.keep_alive || ''} onChange={(e) => updateModel(i, 'keep_alive', e.target.value || null)} placeholder="e.g. 30m" />
                  </div>
                  <div>
                    <label className={labelCls}>
                      Tags
                      <span className="font-normal text-slate-400 ml-1">(comma-separated)</span>
                    </label>
                    <input
                      className={inputCls}
                      value={(model.tags ?? []).join(', ')}
                      onChange={(e) => updateTags(i, e.target.value)}
                      placeholder="e.g. vision, fast"
                    />
                  </div>
                  <div>
                    <label className={labelCls}>
                      Reasoning Effort
                      <span className="font-normal text-slate-400 ml-1">(GPT-5, o3, Gemini 2.5)</span>
                    </label>
                    <select
                      className={inputCls}
                      value={model.reasoning_effort || ''}
                      onChange={(e) => updateModel(i, 'reasoning_effort', e.target.value || null)}
                    >
                      <option value="">Default</option>
                      <option value="low">Low — faster, cheaper</option>
                      <option value="medium">Medium — balanced</option>
                      <option value="high">High — thorough reasoning</option>
                    </select>
                  </div>
                </div>
              </div>
            );
          })}
          {config.models.length === 0 && (
            <p className="text-xs text-slate-400 text-center py-4">No models configured. Add at least one.</p>
          )}
        </div>
        {credsDirty && (
          <div className="mt-4 flex justify-end">
            <button
              onClick={saveCredentials}
              className="px-3 py-1.5 text-xs font-medium bg-blue-600 text-white rounded-lg hover:bg-blue-700 transition-colors"
            >
              Save API Keys
            </button>
          </div>
        )}
        <p className="mt-3 text-[11px] text-slate-400">
          API keys are stored in <code className="text-[11px]">~/.linggen/credentials.json</code>, not in the config file.
          Click the <Star size={10} className="inline text-amber-500" /> icon on a model to add it to the default fallback chain.
        </p>
      </section>
    </div>
  );
};
