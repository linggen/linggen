import { useState } from 'react';
import { ArrowDown, ArrowUp, X } from 'lucide-react';
import type { NotReady, UseEdit, UseOption, Uses } from '../../lib/modelUses';
import { W, say } from '../../lib/modelsWords.mts';
import { translate } from '../../lib/i18n';
import { hintCls, inputCls, linkBtn } from './shell';

const reasons: Record<NotReady, string> = {
  no_key: W.liveMissingKey,
  not_signed_in: W.notSignedIn,
  provider_sign_in: W.notSignedIn,
  sign_in_expired: W.signInExpired,
  usage_limit: W.usageLimit,
  rate_limited: W.rateLimited,
  quota_used_up: W.quotaUsedUp,
  provider_failing: W.providerFailing,
};

function FallbackName({ id, option }: { id: string; option?: UseOption }) {
  if (option?.state === 'ready') return <span className="font-medium text-slate-600 dark:text-slate-300">{option.label}</span>;
  const reason = option ? (option.reason ? reasons[option.reason] : W.notReady) : W.modelMissing;
  return <span className="text-slate-500 dark:text-slate-400">{option?.label ?? id} <span aria-hidden>· </span><span>{reason}</span></span>;
}

const actionCls = 'rounded p-1 text-slate-500 hover:bg-slate-100 hover:text-slate-800 disabled:opacity-35 dark:text-slate-400 dark:hover:bg-white/10 dark:hover:text-slate-200';

export function ChatFallbacks({ view, busy, pick }: { view: Uses; busy: boolean; pick: (edit: UseEdit) => void }) {
  const [open, setOpen] = useState(false);
  const ids = view.chain.slice(1);
  const options = new Map(view.chat.options.map((o) => [o.id, o]));
  const addable = view.chat.options.filter((o) => o.state === 'ready' && !view.chain.includes(o.id));
  const save = (choices: string[]) => pick({ use: 'chat_fallbacks', choices });
  const move = (from: number, to: number) => {
    const next = [...ids];
    [next[from], next[to]] = [next[to], next[from]];
    save(next);
  };
  const actionLabel = (word: typeof W.moveUp | typeof W.moveDown | typeof W.removeFallback, id: string) =>
    say(translate(word) as typeof word ?? word, { model: options.get(id)?.label ?? id });

  return <div className={`${hintCls} mt-1.5 min-w-0`}>
    <div className="flex items-baseline gap-2">
      <div data-testid="chat-fallback-summary" className="min-w-0 truncate">
        {ids.length ? <><span>{W.fallbackLabel}</span>{' '}{ids.map((id, index) => <span key={id}>{index > 0 && ' → '}<FallbackName id={id} option={options.get(id)} /></span>)}</> : <span>{W.noFallbacks}</span>}
      </div>
      <button type="button" className={`${linkBtn} shrink-0`} aria-expanded={open} aria-controls="chat-fallback-editor" disabled={!view.chat.choice} onClick={() => setOpen(!open)}>{open ? W.close : W.editFallbacks}</button>
    </div>
    <div id="chat-fallback-editor" hidden={!open} className="mt-1.5 space-y-1 rounded-lg border border-slate-100 px-2 py-1.5 dark:border-white/10">
      {ids.length > 0 && <ol className="divide-y divide-slate-100 dark:divide-white/5">
        {ids.map((id, index) => <li key={id} data-testid="chat-fallback" data-model={id} className="flex items-center justify-between gap-2 py-0.5">
          <span className="min-w-0 break-words"><FallbackName id={id} option={options.get(id)} /></span>
          <span className="flex shrink-0 items-center">
            <button type="button" className={actionCls} aria-label={actionLabel(W.moveUp, id)} disabled={busy || index === 0} onClick={() => move(index, index - 1)}><ArrowUp size={13} /></button>
            <button type="button" className={actionCls} aria-label={actionLabel(W.moveDown, id)} disabled={busy || index === ids.length - 1} onClick={() => move(index, index + 1)}><ArrowDown size={13} /></button>
            <button type="button" className={actionCls} aria-label={actionLabel(W.removeFallback, id)} disabled={busy} onClick={() => save(ids.filter((_, i) => i !== index))}><X size={13} /></button>
          </span>
        </li>)}
      </ol>}
      <select aria-label={W.addFallback} className={`${inputCls} max-w-full px-2 py-1 dark:[color-scheme:dark]`} value="" disabled={busy || !addable.length} onChange={(e) => save([...ids, e.target.value])}>
        <option value="" disabled>{W.addFallback}</option>
        {addable.map((o) => <option key={o.id} value={o.id}>{o.label}</option>)}
      </select>
    </div>
    {ids.length > 0 && <label className="mt-1 flex w-fit items-center gap-1.5">
      <input type="checkbox" role="switch" aria-label={translate(W.fallbackSwitch) ?? W.fallbackSwitch} checked={view.model_fallback} disabled={busy} className="accent-blue-600" onChange={(e) => pick({ use: 'model_fallback', enabled: e.target.checked })} />
      <span>{view.model_fallback ? W.fallbackSwitch : W.fallbacksOff}</span>
    </label>}
  </div>;
}
