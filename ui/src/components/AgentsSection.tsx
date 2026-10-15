/**
 * Settings → Agents: Ling and Yinyue side by side, each with the model she
 * runs on. Ling's model is the first entry of `routing.default_models` (the
 * star chain in Models — the rest stay her fallbacks); Yinyue's lives in
 * `[pet]`, the on-disk key kept for old files and paired phones.
 */
import React from 'react';
import type { AppConfig } from '../types';
import { useServerStore } from '../stores/serverStore';
import { useAgentPresent } from '../hooks/useAgentPresent';
import { COMPANION_ID } from '../lib/agentsMet.mts';
import { Field, Grid, NumberInput, Section, inputCls } from './settings/kit';

const VOICES = ['vivian', 'serena', 'ono_anna', 'sohee', 'uncle_fu', 'ryan', 'aiden', 'eric', 'dylan'];

type ModelOption = { id: string; authOk: boolean };

/** Models an agent can be put on: the engine's built-ins plus the configured
 *  ones, so no option names an id that isn't wired up. A built-in without its
 *  sign-in stays listed but says so. */
function useModelOptions(config: AppConfig): ModelOption[] {
  const runtimeModels = useServerStore((s) => s.runtimeModels);
  React.useEffect(() => { useServerStore.getState().refreshRuntimeModels(); }, []);
  return React.useMemo(() => {
    const builtins = runtimeModels
      .filter((m) => m.is_builtin && m.id)
      .map((m) => ({ id: m.id, authOk: m.auth_ok !== false }));
    const ids = new Set(builtins.map((b) => b.id));
    const configured = (config.models ?? []).filter((m) => !ids.has(m.id)).map((m) => ({ id: m.id, authOk: true }));
    return [...builtins, ...configured];
  }, [runtimeModels, config.models]);
}

const ModelOptions: React.FC<{ options: ModelOption[]; current?: string }> = ({ options, current }) => (
  <>
    {current && !options.some((o) => o.id === current) && <option value={current}>{current}</option>}
    {options.map((m) => (
      <option key={m.id} value={m.id}>{m.id}{m.authOk ? '' : ' — sign in required'}</option>
    ))}
  </>
);

const AgentCard: React.FC<{ name: string; role: string; children: React.ReactNode }> = ({ name, role, children }) => (
  <div className="rounded-lg border border-slate-100 dark:border-white/5 bg-slate-50/60 dark:bg-white/[0.02] p-4">
    <div className="flex items-baseline gap-2 mb-3">
      <span className="text-sm font-semibold text-slate-800 dark:text-slate-100">{name}</span>
      <span className="text-[11px] text-slate-500 dark:text-slate-400">{role}</span>
    </div>
    {children}
  </div>
);

export const AgentsSection: React.FC<{
  config: AppConfig;
  onChange: (config: AppConfig) => void;
}> = ({ config, onChange }) => {
  const options = useModelOptions(config);
  const yinyueHere = useAgentPresent(COMPANION_ID);
  const chain = config.routing?.default_models ?? [];
  const lingModel = chain[0] ?? '';
  const her = config.pet ?? {};
  const setHer = (patch: Partial<NonNullable<AppConfig['pet']>>) =>
    onChange({ ...config, pet: { ...config.pet, ...patch } });
  // Ling's pick goes to the front of the chain; the others stay her fallbacks.
  const setLingModel = (id: string) =>
    onChange({ ...config, routing: { ...config.routing, default_models: [id, ...chain.filter((m) => m !== id)] } });

  return (
    <Section title="Agents" desc="Each agent runs on its own model.">
      <div className="space-y-4">
        <AgentCard name="Ling" role="Runs your apps and does the work.">
          <Grid>
            <Field
              label="Model"
              hint="Default for new chats. A chat can switch its own."
              details="The first starred model in Models. The other stars are fallbacks, tried in order."
            >
              <select className={inputCls} value={lingModel} onChange={(e) => setLingModel(e.target.value)}>
                {!lingModel && <option value="">Not set (first configured model)</option>}
                <ModelOptions options={options} current={lingModel} />
              </select>
            </Field>
          </Grid>
        </AgentCard>

        {yinyueHere && (
          <AgentCard name="Yinyue" role="Your companion. Remembers you, plays alongside.">
            <Grid>
              <Field label="Yinyue active" hint="Off: she stays silent — no reactions, no voice." details="Hiding her on screen takes a refresh.">
                <select className={inputCls} value={(her.enabled ?? true) ? 'on' : 'off'} onChange={(e) => setHer({ enabled: e.target.value === 'on' })}>
                  <option value="on">On</option>
                  <option value="off">Off</option>
                </select>
              </Field>
              <Field
                label="Model"
                hint="Auto uses Linggen Cloud when signed in, else your default."
                details="Pick a fast model for snappier replies. Applied on her next turn."
              >
                <select className={inputCls} value={her.model ?? 'auto'} onChange={(e) => setHer({ model: e.target.value })}>
                  <option value="auto">Auto</option>
                  <ModelOptions options={options} current={her.model === 'auto' ? undefined : her.model} />
                </select>
              </Field>
              <Field label="Voice" hint="Her speaker. Applies on her next spoken line." details="A Qwen3-TTS speaker. The fallback voice ignores it.">
                <select className={inputCls} value={her.voice ?? 'vivian'} onChange={(e) => setHer({ voice: e.target.value })}>
                  {VOICES.map((v) => <option key={v} value={v}>{v}</option>)}
                </select>
              </Field>
              <Field label="Mute voice" hint="This Mac only. Muted, she still writes every line." details={<>Also <code>/mute</code> and <code>/unmute</code> in any chat.</>}>
                <select className={inputCls} value={her.muted ? 'muted' : 'on'} onChange={(e) => setHer({ muted: e.target.value === 'muted' })}>
                  <option value="on">Voice on</option>
                  <option value="muted">Muted — text only</option>
                </select>
              </Field>
              <Field label="Recall count" hint="Memories she gets each turn. Default 1." details="Kept tight so she stays quick. Range 1–20.">
                <NumberInput integer min={1} max={20} value={her.recall_count ?? 1} onCommit={(v) => setHer({ recall_count: v })} />
              </Field>
              <Field label="Threshold score" hint="Lowest match a memory needs to reach her. Default 0.8." details="Higher means sharper, fewer memories. Range 0–1.">
                <NumberInput min={0} max={1} step={0.05} value={her.recall_min_score ?? 0.8} onCommit={(v) => setHer({ recall_min_score: v })} />
              </Field>
            </Grid>
          </AgentCard>
        )}
      </div>
    </Section>
  );
};
