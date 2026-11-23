/**
 * 「Default model」: the chat default over the chat models that are ready —
 * picking one puts it first and keeps the chain after it as fallbacks — and
 * the other uses (voice, dictation, drawing) as facts from the engine's list.
 */
import React from 'react';
import { W } from '../../lib/modelsWords.mts';
import { chatChoices, useLines, type Provider } from '../../lib/providers.mts';
import { cardCls, hintCls, inputCls } from './shell';

type Props = { providers: Provider[]; first: string | undefined; busy: boolean; onPick: (modelId: string) => void };

export const DefaultCard: React.FC<Props> = ({ providers, first, busy, onPick }) => {
  const choices = chatChoices(providers);
  const groups = [...new Set(choices.map((c) => c.group))];
  const known = !first || choices.some((c) => c.id === first);
  const others = useLines(providers);
  return (
    <section className={cardCls}>
      <h2 className="text-xs font-bold uppercase tracking-wider text-slate-700 dark:text-slate-300">{W.defaultModel}</h2>
      <p className={`${hintCls} mt-1`}>{W.defaultModelHint}</p>
      <div className="mt-3 flex flex-col gap-1.5 sm:flex-row sm:flex-wrap sm:items-center sm:gap-x-4">
        <span className="w-28 shrink-0 text-[13px] font-semibold">{W.chat}</span>
        <select
          data-testid="default-chat"
          aria-label={W.defaultModel}
          className={`${inputCls} min-w-0 max-w-full flex-1 basis-64`}
          value={first ?? ''}
          disabled={busy}
          onChange={(e) => e.target.value && onPick(e.target.value)}
        >
          {!first && <option value="">{W.notSet}</option>}
          {!known && <option value={first}>{first}</option>}
          {groups.map((g) => (
            <optgroup key={g} label={g}>
              {choices.filter((c) => c.group === g).map((c) => <option key={c.id} value={c.id}>{c.text}</option>)}
            </optgroup>
          ))}
        </select>
      </div>
      {others.map((u) => (
        <div key={`${u.word}${u.model}`} data-testid="use-line" className="mt-2 flex flex-col gap-0.5 sm:flex-row sm:flex-wrap sm:items-center sm:gap-x-4">
          <span className="w-28 shrink-0 text-[13px] font-semibold">{u.word}</span>
          <span className="min-w-0 break-all text-xs" data-no-i18n>{u.model}</span>
          <span className={hintCls}>{u.ready ? W.ready : W.notReady}</span>
        </div>
      ))}
    </section>
  );
};
