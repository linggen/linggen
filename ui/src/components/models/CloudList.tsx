/**
 * Linggen Cloud's models: a checkbox each, as the site lists them (the engine
 * keeps the last answer). A tick files an ordinary model row; 首选/备选 stay
 * on the 聊天 card. Unticking the first default is refused by the engine
 * until another default is picked there (「Pick another default model first.」).
 */
import React from 'react';
import { providersApi } from '../../lib/endpoints';
import { W } from '../../lib/modelsWords.mts';
import type { Provider } from '../../lib/providers.mts';
import { hintCls } from './shell';
import type { Run } from './useProviders';

export const CloudList: React.FC<{ provider: Provider; run: Run; busy: boolean }> = ({ provider, run, busy }) => {
  const lines = provider.cloud ?? [];
  if (!lines.length) return <p className={hintCls}>{W.noModelsYet}</p>;
  return (
    <div data-testid="cloud-list">
      {lines.map((l) => (
        <label key={l.model} data-testid="cloud-model" className="flex items-center gap-2 py-1 text-xs font-semibold">
          <input
            type="checkbox"
            className="h-3.5 w-3.5 shrink-0 accent-blue-600"
            checked={!!l.row}
            disabled={busy}
            onChange={() => void run(() => providersApi.tickCloud(l.model, !l.row))}
          />
          <span data-no-i18n>{l.label}</span>
        </label>
      ))}
    </div>
  );
};
