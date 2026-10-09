import React from 'react';
import type { AppConfig } from '../types';
import { useServerStore } from '../stores/serverStore';

const inputCls = 'w-full bg-slate-100 dark:bg-white/5 border border-slate-200 dark:border-white/10 rounded-lg px-3 py-2 text-xs outline-none focus:ring-1 focus:ring-blue-500/50';
const labelCls = 'text-[11px] font-bold uppercase tracking-wider text-slate-500 dark:text-slate-400';
const sectionCls = 'bg-white dark:bg-[#141414] rounded-xl border border-slate-200 dark:border-white/5 shadow-sm p-5';

/** Her settings — voice, model, recall. Rendered only once she is met
 *  (`GeneralTab` leaves it out until then). */
export const PetSection: React.FC<{
  config: AppConfig;
  onChange: (config: AppConfig) => void;
}> = ({ config, onChange }) => {
  // Pet model picker: the built-in models (runtime-injected — ChatGPT
  // gpt-6 family, Linggen Cloud) plus the user's actually-configured
  // models, so we never offer an id that isn't wired up (an unconfigured
  // pick fails to resolve). "auto" is added directly in the <select>.
  const runtimeModels = useServerStore((s) => s.runtimeModels);
  React.useEffect(() => { useServerStore.getState().refreshRuntimeModels(); }, []);
  const builtins = React.useMemo(
    () => runtimeModels.filter((m) => m.is_builtin && m.id).map((m) => ({ id: m.id, authOk: m.auth_ok !== false })),
    [runtimeModels],
  );
  // Built-ins that lack their sign-in stay listed but say so — a bare id
  // here reads as a working brain, and picking it silences her entirely.
  const petModelOptions = React.useMemo(() => {
    const builtinIds = builtins.map((b) => b.id);
    const configured = (config.models ?? []).map((m) => ({ id: m.id, authOk: true }));
    return [...builtins, ...configured.filter((m) => !builtinIds.includes(m.id))];
  }, [config.models, builtins]);

  return (
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
            {petModelOptions.map((m) => (
              <option key={m.id} value={m.id}>{m.id}{m.authOk ? '' : ' — sign in required'}</option>
            ))}
          </select>
          <p className="text-[11px] text-slate-400 mt-0.5">Her brain. Auto uses the Linggen Cloud model when signed in, else your default. Pick a fast configured model for snappier replies. Applied per turn.</p>
        </div>
        <div>
          <label className={labelCls}>Voice</label>
          <select
            className={inputCls}
            value={config.pet?.voice ?? 'vivian'}
            onChange={(e) => onChange({ ...config, pet: { ...config.pet, voice: e.target.value } })}
          >
            {['vivian', 'serena', 'ono_anna', 'sohee', 'uncle_fu', 'ryan', 'aiden', 'eric', 'dylan'].map((v) => (
              <option key={v} value={v}>{v}</option>
            ))}
          </select>
          <p className="text-[11px] text-slate-400 mt-0.5">Her speaker (Qwen3-TTS). Applied on her next spoken line; the fallback voice ignores it.</p>
        </div>
        <div>
          <label className={labelCls}>Mute Voice</label>
          <select
            className={inputCls}
            value={config.pet?.muted ? 'muted' : 'on'}
            onChange={(e) => onChange({ ...config, pet: { ...config.pet, muted: e.target.value === 'muted' } })}
          >
            <option value="on">Voice on</option>
            <option value="muted">Muted — text only</option>
          </select>
          <p className="text-[11px] text-slate-400 mt-0.5">On this Mac. Muted, she still writes every line. Also <code>/mute</code> and <code>/unmute</code> in any chat.</p>
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
  );
};
