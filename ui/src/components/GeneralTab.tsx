import React from 'react';
import type { AppConfig } from '../types';

const inputCls = 'w-full bg-slate-100 dark:bg-white/5 border border-slate-200 dark:border-white/10 rounded-lg px-3 py-2 text-xs outline-none focus:ring-1 focus:ring-blue-500/50';
const labelCls = 'text-[11px] font-bold uppercase tracking-wider text-slate-500 dark:text-slate-400';
const sectionCls = 'bg-white dark:bg-[#141414] rounded-xl border border-slate-200 dark:border-white/5 shadow-sm p-5';

export const GeneralTab: React.FC<{
  config: AppConfig;
  onChange: (config: AppConfig) => void;
}> = ({ config, onChange }) => {
  // Draft text for the threshold field so partial input (e.g. typing the
  // first digit "9" of "95") is not reverted by the controlled value /
  // range check. Commit valid values live; normalize on blur.
  const thresholdFromConfig = () =>
    config.agent.compact_threshold != null
      ? String(Math.round(config.agent.compact_threshold * 100))
      : '';
  const [thresholdText, setThresholdText] = React.useState(thresholdFromConfig);
  React.useEffect(() => {
    setThresholdText(thresholdFromConfig());
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [config.agent.compact_threshold]);

  // Pet model picker: the built-in models (runtime-injected — ChatGPT
  // gpt-5.6 family, Linggen Cloud) plus the user's actually-configured
  // models, so we never offer an id that isn't wired up (an unconfigured
  // pick fails to resolve). "auto" is added directly in the <select>.
  const [builtinIds, setBuiltinIds] = React.useState<string[]>([]);
  React.useEffect(() => {
    fetch('/api/models')
      .then((r) => r.json())
      .then((ms) => {
        if (Array.isArray(ms)) {
          setBuiltinIds(ms.filter((m) => m?.is_builtin && m.id).map((m) => m.id));
        }
      })
      .catch(() => {});
  }, []);
  const petModelOptions = React.useMemo(() => {
    const configured = (config.models ?? []).map((m) => m.id);
    return [...builtinIds, ...configured.filter((id) => !builtinIds.includes(id))];
  }, [config.models, builtinIds]);

  return (
    <div className="space-y-6">
      {/* Agent Settings */}
      <section className={sectionCls}>
        <h2 className="text-xs font-bold uppercase tracking-wider text-slate-700 dark:text-slate-300 mb-4">Agent Settings</h2>
        <div className="grid grid-cols-2 gap-4">
          <div>
            <label className={labelCls}>Max Iterations</label>
            <input
              className={inputCls}
              type="number"
              min={1}
              value={config.agent.max_iters}
              onChange={(e) => onChange({ ...config, agent: { ...config.agent, max_iters: parseInt(e.target.value) || 1 } })}
            />
          </div>
          <div>
            <label className={labelCls}>Write Safety Mode</label>
            <select
              className={inputCls}
              value={config.agent.write_safety_mode}
              onChange={(e) => onChange({ ...config, agent: { ...config.agent, write_safety_mode: e.target.value } })}
            >
              <option value="strict">strict</option>
              <option value="warn">warn</option>
              <option value="off">off</option>
            </select>
          </div>
          <div>
            <label className={labelCls}>Default Permission Mode</label>
            <select
              className={inputCls}
              value={config.agent.tool_permission_mode}
              onChange={(e) => onChange({ ...config, agent: { ...config.agent, tool_permission_mode: e.target.value } })}
            >
              <option value="ask">read (default)</option>
              <option value="accept_edits">edit</option>
              <option value="auto">admin</option>
            </select>
            <p className="text-[11px] text-slate-400 mt-0.5">Default mode for new sessions. Per-session mode can be changed in the chat header.</p>
          </div>
          <div className="col-span-2">
            <label className={labelCls}>Prompt Loop Breaker</label>
            <textarea
              className={`${inputCls} min-h-[60px] resize-y`}
              value={config.agent.prompt_loop_breaker || ''}
              onChange={(e) => onChange({ ...config, agent: { ...config.agent, prompt_loop_breaker: e.target.value || null } })}
              placeholder="(optional) Custom prompt to break tool loops"
            />
          </div>
          <div>
            <label className={labelCls}>Auto-Compact Threshold (%)</label>
            <input
              className={inputCls}
              type="number"
              min={10}
              max={99}
              step={1}
              value={thresholdText}
              onChange={(e) => {
                const raw = e.target.value;
                setThresholdText(raw);
                if (raw === '') {
                  onChange({ ...config, agent: { ...config.agent, compact_threshold: null } });
                  return;
                }
                const pct = parseInt(raw, 10);
                if (Number.isFinite(pct) && pct >= 10 && pct <= 99) {
                  onChange({ ...config, agent: { ...config.agent, compact_threshold: pct / 100 } });
                }
                // partial / out-of-range: keep showing what was typed,
                // don't commit — normalized on blur.
              }}
              onBlur={() => {
                const pct = parseInt(thresholdText, 10);
                if (!Number.isFinite(pct)) {
                  setThresholdText('');
                  onChange({ ...config, agent: { ...config.agent, compact_threshold: null } });
                  return;
                }
                const clamped = Math.min(99, Math.max(10, pct));
                setThresholdText(String(clamped));
                onChange({ ...config, agent: { ...config.agent, compact_threshold: clamped / 100 } });
              }}
              placeholder="95 (default)"
            />
            <p className="text-[11px] text-slate-400 mt-0.5">Auto-compact fires when context usage crosses this fraction of the model window. Blank = engine default (95%). Per-session overrides (e.g. Pulse at 50%) still take precedence. Applies to new sessions.</p>
          </div>
          <div>
            <label className={labelCls}>Memory Inject Score</label>
            <input
              className={inputCls}
              type="number"
              min={0}
              max={1}
              step={0.05}
              value={config.agent.memory_inject_min_score ?? 0.7}
              onChange={(e) => {
                const v = parseFloat(e.target.value);
                if (Number.isFinite(v) && v >= 0 && v <= 1) {
                  onChange({ ...config, agent: { ...config.agent, memory_inject_min_score: v } });
                }
              }}
              placeholder="0.7 (default)"
            />
            <p className="text-[11px] text-slate-400 mt-0.5">Per-row relevance floor for per-turn auto-recall — cosine similarity plus a keyword-match boost. Any row below this is dropped — never injected, never shown. Raise for stricter, fewer hits; lower to let weaker matches through. Range 0–1. Default 0.7.</p>
          </div>
          <div>
            <label className={labelCls}>Recall Count</label>
            <input
              className={inputCls}
              type="number"
              min={1}
              max={20}
              step={1}
              value={config.agent.memory_recall_count ?? 3}
              onChange={(e) => {
                const v = parseInt(e.target.value, 10);
                if (Number.isFinite(v) && v >= 1 && v <= 20) {
                  onChange({ ...config, agent: { ...config.agent, memory_recall_count: v } });
                }
              }}
              placeholder="3 (default)"
            />
            <p className="text-[11px] text-slate-400 mt-0.5">How many recalled memories are injected per turn — the top matches after project filtering. Range 1–20. Default 3.</p>
          </div>
          <div className="col-span-2">
            <label className={labelCls}>Ling-mem URL</label>
            <input
              className={inputCls}
              value={config.agent.ling_mem_url ?? 'http://127.0.0.1:9888'}
              onChange={(e) => onChange({ ...config, agent: { ...config.agent, ling_mem_url: e.target.value } })}
              placeholder="http://127.0.0.1:9888"
            />
            <p className="text-[11px] text-slate-400 mt-0.5">Base URL of the local <code>ling-mem</code> HTTP daemon. The engine's built-in <code>Memory_query</code> / <code>Memory_write</code> tools dispatch here, and the <code>dream</code> mission fetches <code>episodic_ttl_days</code> from <code>&lt;url&gt;/api/config</code>. Only change if you ran <code>ling-mem start</code> on a non-default port or pointed it at a remote host.</p>
          </div>
        </div>
      </section>

      {/* Pet */}
      <section className={sectionCls}>
        <h2 className="text-xs font-bold uppercase tracking-wider text-slate-700 dark:text-slate-300 mb-4">Pet</h2>
        <div className="grid grid-cols-2 gap-4">
          <div>
            <label className={labelCls}>Enabled</label>
            <select
              className={inputCls}
              value={(config.pet?.enabled ?? true) ? 'on' : 'off'}
              onChange={(e) => onChange({ ...config, pet: { ...config.pet, enabled: e.target.value === 'on' } })}
            >
              <option value="on">On</option>
              <option value="off">Off (disable)</option>
            </select>
            <p className="text-[11px] text-slate-400 mt-0.5">When off, the pet stays silent — no reactions, no voice. Hiding the avatar takes a refresh.</p>
          </div>
          <div>
            <label className={labelCls}>Pet</label>
            <select
              className={inputCls}
              value={config.pet?.pet ?? 'yinyue'}
              onChange={(e) => onChange({ ...config, pet: { ...config.pet, pet: e.target.value } })}
            >
              <option value="yinyue">Yinyue</option>
            </select>
            <p className="text-[11px] text-slate-400 mt-0.5">Which companion to show. More avatars later.</p>
          </div>
          <div>
            <label className={labelCls}>Model</label>
            <select
              className={inputCls}
              value={config.pet?.model ?? 'auto'}
              onChange={(e) => onChange({ ...config, pet: { ...config.pet, model: e.target.value } })}
            >
              <option value="auto">Auto (cloud default / your model)</option>
              {petModelOptions.map((id) => (
                <option key={id} value={id}>{id}</option>
              ))}
            </select>
            <p className="text-[11px] text-slate-400 mt-0.5">Her brain. Auto uses the Linggen Cloud model when signed in, else your default. Pick a fast configured model for snappier replies. Applied per turn.</p>
          </div>
          <div>
            <label className={labelCls}>Speech Text</label>
            <select
              className={inputCls}
              value={(config.pet?.show_text ?? true) ? 'on' : 'off'}
              onChange={(e) => onChange({ ...config, pet: { ...config.pet, show_text: e.target.value === 'on' } })}
            >
              <option value="on">Show bubble</option>
              <option value="off">Voice only</option>
            </select>
            <p className="text-[11px] text-slate-400 mt-0.5">Whether her spoken line also shows as an on-screen bubble.</p>
          </div>
          <div>
            <label className={labelCls}>Recall Count</label>
            <input
              className={inputCls}
              type="number"
              min={1}
              max={20}
              step={1}
              value={config.pet?.recall_count ?? 1}
              onChange={(e) => {
                const v = parseInt(e.target.value, 10);
                if (Number.isFinite(v) && v >= 1 && v <= 20) {
                  onChange({ ...config, pet: { ...config.pet, recall_count: v } });
                }
              }}
              placeholder="1 (default)"
            />
            <p className="text-[11px] text-slate-400 mt-0.5">Memories injected per pet turn. Kept tight (1) so she stays snappy. Range 1–20.</p>
          </div>
          <div>
            <label className={labelCls}>Threshold Score</label>
            <input
              className={inputCls}
              type="number"
              min={0}
              max={1}
              step={0.05}
              value={config.pet?.recall_min_score ?? 0.8}
              onChange={(e) => {
                const v = parseFloat(e.target.value);
                if (Number.isFinite(v) && v >= 0 && v <= 1) {
                  onChange({ ...config, pet: { ...config.pet, recall_min_score: v } });
                }
              }}
              placeholder="0.8 (default)"
            />
            <p className="text-[11px] text-slate-400 mt-0.5">Min cosine score for an injected memory — higher = sharper, fewer hits. Range 0–1. Default 0.8.</p>
          </div>
        </div>
      </section>

      {/* Server */}
      <section className={sectionCls}>
        <h2 className="text-xs font-bold uppercase tracking-wider text-slate-700 dark:text-slate-300 mb-4">Server</h2>
        <div className="grid grid-cols-2 gap-4">
          <div>
            <label className={labelCls}>Host</label>
            <input
              className={inputCls}
              value={config.server.host}
              onChange={(e) => onChange({ ...config, server: { ...config.server, host: e.target.value } })}
              placeholder="127.0.0.1"
            />
            <p className="text-[11px] text-slate-400 mt-1">Use 0.0.0.0 to allow LAN access.</p>
          </div>
          <div>
            <label className={labelCls}>Port</label>
            <input
              className={inputCls}
              type="number"
              min={1}
              max={65535}
              value={config.server.port}
              onChange={(e) => onChange({ ...config, server: { ...config.server, port: parseInt(e.target.value) || 8080 } })}
            />
          </div>
        </div>
        <p className="text-[11px] text-amber-500 mt-2">Changing host or port requires a server restart to take effect.</p>
      </section>

      {/* Logging */}
      <section className={sectionCls}>
        <h2 className="text-xs font-bold uppercase tracking-wider text-slate-700 dark:text-slate-300 mb-4">Logging</h2>
        <div className="grid grid-cols-3 gap-4">
          <div>
            <label className={labelCls}>Level</label>
            <select
              className={inputCls}
              value={config.logging.level || ''}
              onChange={(e) => onChange({ ...config, logging: { ...config.logging, level: e.target.value || null } })}
            >
              <option value="">Default</option>
              <option value="trace">trace</option>
              <option value="debug">debug</option>
              <option value="info">info</option>
              <option value="warn">warn</option>
              <option value="error">error</option>
            </select>
          </div>
          <div>
            <label className={labelCls}>Directory</label>
            <input
              className={inputCls}
              value={config.logging.directory || ''}
              onChange={(e) => onChange({ ...config, logging: { ...config.logging, directory: e.target.value || null } })}
              placeholder="(default)"
            />
          </div>
          <div>
            <label className={labelCls}>Retention Days</label>
            <input
              className={inputCls}
              type="number"
              min={1}
              value={config.logging.retention_days ?? ''}
              onChange={(e) => onChange({ ...config, logging: { ...config.logging, retention_days: e.target.value ? parseInt(e.target.value) : null } })}
              placeholder="(default)"
            />
          </div>
        </div>
      </section>
    </div>
  );
};
