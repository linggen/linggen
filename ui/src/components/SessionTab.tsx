/**
 * Settings → Session: how a new chat behaves (permissions, follow-ups,
 * compaction, with the rarely-touched knobs folded away) and what memory it
 * recalls. Keys live under `[agent]`; only the grouping is new.
 */
import React from 'react';
import type { AppConfig } from '../types';
import { Field, Grid, MoreGroup, NumberInput, Section, inputCls } from './settings/kit';

type AgentCfg = AppConfig['agent'];

const CompactThreshold: React.FC<{ value: number | null | undefined; onCommit: (v: number | null) => void }> = ({ value, onCommit }) => {
  // Draft text so a partial entry (the "9" of "95") is not reverted by the
  // range check. Valid values commit live; blur normalizes.
  const fromValue = () => (value != null ? String(Math.round(value * 100)) : '');
  const [text, setText] = React.useState(fromValue);
  // eslint-disable-next-line react-hooks/exhaustive-deps
  React.useEffect(() => setText(fromValue()), [value]);
  return (
    <input
      className={inputCls}
      type="number"
      min={10}
      max={99}
      step={1}
      value={text}
      placeholder="95 (default)"
      onChange={(e) => {
        const raw = e.target.value;
        setText(raw);
        if (raw === '') return onCommit(null);
        const pct = parseInt(raw, 10);
        if (Number.isFinite(pct) && pct >= 10 && pct <= 99) onCommit(pct / 100);
      }}
      onBlur={() => {
        const pct = parseInt(text, 10);
        if (!Number.isFinite(pct)) { setText(''); return onCommit(null); }
        const clamped = Math.min(99, Math.max(10, pct));
        setText(String(clamped));
        onCommit(clamped / 100);
      }}
    />
  );
};

export const SessionTab: React.FC<{
  config: AppConfig;
  onChange: (config: AppConfig) => void;
}> = ({ config, onChange }) => {
  const a = config.agent;
  const set = (patch: Partial<AgentCfg>) => onChange({ ...config, agent: { ...a, ...patch } });

  return (
    <div className="space-y-6">
      <Section title="Session" desc="Defaults for new chats. A chat can change its own.">
        <Grid>
          <Field label="Permission mode" hint="What a new chat may do without asking." details="Change it per chat from the chat header.">
            <select className={inputCls} value={a.tool_permission_mode} onChange={(e) => set({ tool_permission_mode: e.target.value })}>
              <option value="ask">Read (default)</option>
              <option value="accept_edits">Edit</option>
              <option value="auto">Admin</option>
            </select>
          </Field>
          <Field label="Suggest follow-ups" hint="After a reply, buttons with what you might ask next.">
            <select className={inputCls} value={a.suggest_followups === false ? 'off' : 'on'} onChange={(e) => set({ suggest_followups: e.target.value === 'on' })}>
              <option value="on">On</option>
              <option value="off">Off</option>
            </select>
          </Field>
          <Field
            label="Auto-compact at (%)"
            hint="Summarize the chat when context fills this much. Blank = 95%."
            details="A fraction of the model's window. A chat's own setting (Pulse uses 50%) wins. Applies to new chats."
          >
            <CompactThreshold value={a.compact_threshold} onCommit={(v) => set({ compact_threshold: v })} />
          </Field>
        </Grid>
        <MoreGroup label="Advanced">
          <Grid>
            <Field label="Max iterations" hint="Most tool steps in one reply before it stops.">
              <NumberInput integer min={1} value={a.max_iters} onCommit={(v) => set({ max_iters: v })} />
            </Field>
            <Field label="Write safety" hint="Guard on overwriting files the agent hasn't read." details="Strict blocks, warn lets it through with a warning, off skips the check.">
              <select className={inputCls} value={a.write_safety_mode} onChange={(e) => set({ write_safety_mode: e.target.value })}>
                <option value="strict">Strict</option>
                <option value="warn">Warn</option>
                <option value="off">Off</option>
              </select>
            </Field>
            <Field label="Loop breaker prompt" hint="Sent when the agent repeats the same tool call. Blank = built-in." wide>
              <textarea
                className={`${inputCls} min-h-[60px] resize-y`}
                value={a.prompt_loop_breaker || ''}
                onChange={(e) => set({ prompt_loop_breaker: e.target.value || null })}
                placeholder="(optional)"
              />
            </Field>
          </Grid>
        </MoreGroup>
      </Section>

      <Section title="Memory" desc="What Ling recalls each turn. Yinyue's recall is set under Agents.">
        <Grid>
          <Field label="Recall count" hint="Memories added to each turn. Default 3." details="The top matches after project filtering. Range 1–20.">
            <NumberInput integer min={1} max={20} value={a.memory_recall_count ?? 3} onCommit={(v) => set({ memory_recall_count: v })} />
          </Field>
          <Field
            label="Inject score"
            hint="Lowest match a memory needs to be added. Default 0.7."
            details="Cosine similarity plus a keyword boost. Below it, a memory is never shown. Range 0–1."
          >
            <NumberInput min={0} max={1} step={0.05} value={a.memory_inject_min_score ?? 0.7} onCommit={(v) => set({ memory_inject_min_score: v })} />
          </Field>
        </Grid>
        <MoreGroup label="Advanced">
          <Grid>
            <Field
              label="Ling-mem URL"
              hint="Where the memory service runs. Change only for another port or host."
              details={<>The memory MCP server is <code>&lt;url&gt;/mcp</code>; the dream mission reads <code>&lt;url&gt;/api/config</code>.</>}
              wide
            >
              <input
                className={inputCls}
                value={a.ling_mem_url ?? 'http://127.0.0.1:9528'}
                onChange={(e) => set({ ling_mem_url: e.target.value })}
                placeholder="http://127.0.0.1:9528"
              />
            </Field>
          </Grid>
        </MoreGroup>
      </Section>
    </div>
  );
};
