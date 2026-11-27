/**
 * Linggen Cloud's models: a checkbox each, as the site lists them (the engine
 * keeps the last answer). A tick files an ordinary model row; 首选/备选 stay
 * on the 聊天 card. Unticking the first default is refused by the engine
 * until another default is picked there (「Pick another default model first.」).
 * Live voice and read-aloud follow, ticked and fixed: every subscription has
 * them, so they cannot be unticked.
 */
import React from 'react';
import { providersApi } from '../../lib/endpoints';
import { W } from '../../lib/modelsWords.mts';
import type { CloudLine, Provider } from '../../lib/providers.mts';
import { hintCls } from './shell';
import type { Run } from './useProviders';

/** The one tick component: a checkbox for each model of a sign-in (Linggen Cloud's, ChatGPT's). */
export const ModelTicks: React.FC<{ lines: CloudLine[]; busy: boolean; onTick: (line: CloudLine) => void }> = ({ lines, busy, onTick }) => (
  <>
    {lines.map((l) => (
      <label key={l.model} data-testid="cloud-model" className="flex items-center gap-2 py-1 text-xs font-semibold">
        <input type="checkbox" className="h-3.5 w-3.5 shrink-0 accent-blue-600" checked={!!l.row} disabled={busy} onChange={() => onTick(l)} />
        <span data-no-i18n>{l.label}</span>
      </label>
    ))}
  </>
);

export const CloudList: React.FC<{ provider: Provider; run: Run; busy: boolean }> = ({ provider, run, busy }) => {
  const lines = provider.cloud ?? [];
  const fixed = provider.models.filter((m) => m.builtin && (m.kind === 'live_voice' || m.kind === 'speak'));
  if (!lines.length && !fixed.length) return <p className={hintCls}>{W.noModelsYet}</p>;
  return (
    <div data-testid="cloud-list">
      <ModelTicks lines={lines} busy={busy} onTick={(l) => void run(() => providersApi.tickCloud(l.model, !l.row))} />
      {fixed.map((m) => (
        <label key={m.id} data-testid="cloud-fixed" title={W.includedAlways} className="flex items-center gap-2 py-1 text-xs font-semibold">
          <input type="checkbox" className="h-3.5 w-3.5 shrink-0 accent-blue-600 opacity-60" checked disabled aria-label={W.includedAlways} />
          <span>{m.kind === 'live_voice' ? W.liveVoice : W.readAloud}</span>
          <span className={hintCls} data-no-i18n>{m.model}</span>
        </label>
      ))}
      {fixed.length > 0 && <p className={`${hintCls} mt-0.5`}>{W.includedAlways}</p>}
    </div>
  );
};
