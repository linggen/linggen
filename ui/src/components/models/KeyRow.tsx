/**
 * One provider the person pays with a key. Under its head: the key (pasted
 * once; the page only ever learns that it is set), the models the person
 * added — one input that suggests ids from the provider's own list — the
 * live-voice lines, and the address of a provider the person typed. The ⋯
 * menu removes the provider. Every action calls its route (lib/endpoints `providersApi`).
 */
import React, { useEffect, useState } from 'react';
import { X } from 'lucide-react';
import { providersApi } from '../../lib/endpoints';
import { confirmDialog } from '../../lib/confirmDialog';
import { W, say } from '../../lib/modelsWords.mts';
import { chatModels, heldDefaults, liveModels, suggestionsFor, type Provider, type Suggestion } from '../../lib/providers.mts';
import { RowMenu, Row, Tag, ghostBtn, hintCls, inputCls, primaryBtn, sectionLabel, sectionRule } from './shell';
import type { Run } from './useProviders';

type Props = {
  provider: Provider;
  open: boolean;
  onToggle: () => void;
  run: Run;
  busy: boolean;
  makeDefault: (modelId: string) => Promise<void>;
  /** The default chat chain, first model first: what the remove confirm names. */
  chain: string[];
};

/** The head's second line: the models added, or what is missing. */
function summary(p: Provider): string {
  if (!p.key?.set) return W.noKeyYet;
  const names = chatModels(p).map((m) => m.name);
  return names.length ? names.join(' · ') : W.noModelsAdded;
}

export const KeyRow: React.FC<Props> = (props) => {
  const { provider: p, open, onToggle, run, busy, chain } = props;
  const removeProvider = async () => {
    const held = heldDefaults(p, chain);
    const lines = [say(W.removeProviderAsk, { provider: p.label })];
    if (held.first) lines.push(W.removeHoldsDefault);
    if (held.fallbacks.length) lines.push(say(W.removeHoldsFallbacks, { n: held.fallbacks.length }));
    if (await confirmDialog(lines.join(' '))) await run(() => providersApi.remove(p.id));
  };
  return (
    <Row testId={p.provider} name={p.label} way={p.way} subtitle={summary(p)} open={open} onToggle={onToggle}
      tags={<>{p.used_by.includes('chat') && <Tag tone="blue">{W.chat}</Tag>}{p.used_by.includes('live_voice') && <Tag tone="blue">{W.liveVoice}</Tag>}</>}
      right={<RowMenu disabled={busy} items={[{ label: W.removeProvider, run: () => void removeProvider() }]} />}
      detail={p.key?.set ? <span>{say(W.billedBy, { provider: p.label })}</span> : <span className="rounded-md border border-slate-200 bg-white px-2 py-0.5 font-semibold dark:border-white/10 dark:bg-white/5">{W.fillKey}</span>}>
      <KeyField {...props} />
      <Models {...props} />
      {!p.listed && <Address {...props} />}
    </Row>
  );
};

/** The key: 已设置密钥 · 更换, or one field to paste it. Removing goes with the provider (⋯). */
const KeyField: React.FC<Props> = ({ provider: p, run, busy }) => {
  const [text, setText] = useState('');
  const [replacing, setReplacing] = useState(false);
  const save = async () => {
    if (!text.trim()) return;
    if (await run(() => providersApi.setKey(p.id, text.trim()))) {
      setText('');
      setReplacing(false);
    }
  };
  if (p.key?.conflict) {
    return (
      <div className="flex flex-wrap items-center gap-2" data-testid="key-conflict">
        <span className="text-xs font-semibold text-amber-700 dark:text-amber-300">{W.keyConflict}</span>
        {p.key.can_keep_model_key
          ? <button type="button" className={ghostBtn} disabled={busy} onClick={() => void run(() => providersApi.keepModelKey(p.id))}>{W.keepModelKey}</button>
          : <span className={hintCls}>{W.keysInUse}</span>}
      </div>
    );
  }
  const set = !!p.key?.set;
  return (
    <div className={`${sectionRule} flex flex-wrap items-center gap-2`}>
      <span className={sectionLabel}>{W.keyLabel}</span>
      {set && !replacing ? (
        <span className={`${hintCls} flex items-center gap-1.5`}>
          <span data-testid="key-set">{W.keySet}</span>
          <span aria-hidden>·</span>
          <button type="button" className="font-semibold text-blue-600 hover:text-blue-700 dark:text-blue-400" disabled={busy} onClick={() => setReplacing(true)}>{W.replaceKey}</button>
        </span>
      ) : (
        <>
          <input
            type="password"
            autoComplete="off"
            data-testid="key-input"
            className={`${inputCls} w-64 max-w-full flex-1`}
            placeholder={W.pasteKey}
            aria-label={W.pasteKey}
            value={text}
            onChange={(e) => setText(e.target.value)}
            onKeyDown={(e) => { if (e.key === 'Enter') void save(); }}
          />
          <button type="button" className={primaryBtn} disabled={busy || !text.trim()} onClick={() => void save()}>{W.saveKey}</button>
        </>
      )}
    </div>
  );
};

/** The models: what was added (✕ removes), one input with suggestions, the live-voice lines. */
const Models: React.FC<Props> = (props) => {
  const { provider: p, run, busy, makeDefault } = props;
  const added = chatModels(p);
  const live = liveModels(p);
  return (
    <div className={`${sectionRule} grid gap-2 sm:grid-cols-[4rem_minmax(0,1fr)]`}>
      <span className={sectionLabel}>{W.models}</span>
      <div className="min-w-0 flex-1">
        {added.map((m) => (
          <div key={m.id} className="flex items-center gap-2 py-1 text-xs" data-testid="added-model">
            <span className="min-w-0 flex-1 truncate font-semibold" data-no-i18n title={m.model}>{m.name}</span>
            <button type="button" className="text-[11px] font-semibold text-blue-600 hover:text-blue-700 disabled:opacity-50 dark:text-blue-400" disabled={busy || !m.ready} onClick={() => void makeDefault(m.id)}>{W.makeDefault}</button>
            <button type="button" aria-label={say(W.removeModelNamed, { model: m.model })} className="rounded p-0.5 text-slate-400 hover:bg-slate-100 hover:text-red-600 disabled:opacity-50 dark:hover:bg-white/10" disabled={busy} onClick={() => void run(() => providersApi.removeModel(p.id, m.id))}>
              <X size={12} />
            </button>
          </div>
        ))}
        {p.key?.set && <ModelInput {...props} />}
        {!p.key?.set && !added.length && <p className={hintCls}>{W.keyFirst}</p>}
        {live.map((m) => (
          <div key={m.id} className="mt-1 flex flex-wrap items-center gap-1.5 text-xs" data-testid="live-model">
            <span className="font-semibold" data-no-i18n>{m.model}</span>
            <Tag>{W.liveTag}</Tag>
            <span className={hintCls}>{m.ready ? W.liveAvailable : W.liveMissingKey}</span>
            <span className={hintCls}>{W.lingyuUses}</span>
          </div>
        ))}
      </div>
    </div>
  );
};

/** Type a few letters: the provider's own ids that match are listed; a click adds one.
 *  A full id + Enter adds it even when the list could not be read. The list is
 *  asked once when the row opens and kept for the page's life. */
const ModelInput: React.FC<Props> = ({ provider: p, run, busy }) => {
  const [typed, setTyped] = useState('');
  const [list, setList] = useState<Suggestion[] | null | undefined>(undefined);
  useEffect(() => {
    let live = true;
    providersApi.suggestions(p.id).then((r) => { if (live) setList(r.models); }).catch(() => { if (live) setList(null); });
    return () => { live = false; };
  }, [p.id]);
  const shown = suggestionsFor(list, typed, p.models.map((m) => m.model));
  const add = async (id: string) => {
    if (!id.trim()) return;
    if (await run(() => providersApi.addModel(p.id, id.trim()))) setTyped('');
  };
  return (
    <div className="relative mt-1" data-testid="model-input">
      <input
        className={`${inputCls} w-72 max-w-full`}
        placeholder={W.modelInput}
        aria-label={W.modelInput}
        value={typed}
        disabled={busy}
        onChange={(e) => setTyped(e.target.value)}
        onKeyDown={(e) => { if (e.key === 'Enter') void add(typed); }}
      />
      {shown.length > 0 && (
        <div role="listbox" className="absolute left-0 top-8 z-10 w-72 max-w-full rounded-lg border border-slate-200 bg-white p-1 shadow-lg dark:border-white/10 dark:bg-[#1c1c1c]" data-testid="model-suggestions">
          {shown.map((s) => (
            <button key={s.id} type="button" role="option" aria-selected={false} disabled={busy} onClick={() => void add(s.id)} className="block w-full truncate rounded px-2.5 py-1.5 text-left text-xs hover:bg-slate-100 dark:hover:bg-white/10" data-no-i18n>
              {s.id}{s.name && s.name !== s.id && <span className="ml-1.5 text-slate-400">{s.name}</span>}
            </button>
          ))}
        </div>
      )}
      {list === null && <p className={`${hintCls} mt-1`}>{W.noSuggestions}</p>}
      {list && typed.trim() && shown.length === 0 && <p data-testid="no-match" className={`${hintCls} mt-1`}>{W.noMatch}</p>}
    </div>
  );
};

const Address: React.FC<Props> = ({ provider: p, run, busy }) => {
  const [url, setUrl] = useState(p.url);
  return (
        <div data-testid="provider-address" className={`${sectionRule} sm:pl-[4.5rem]`}>
          <div className={hintCls}>{W.address}</div>
          <div className="mt-1 flex flex-wrap items-center gap-2">
            <input className={`${inputCls} w-64 max-w-full flex-1 font-mono`} aria-label={W.address} value={url} onChange={(e) => setUrl(e.target.value)} />
            <button type="button" className={ghostBtn} disabled={busy || !url.trim() || url.trim() === p.url} onClick={() => void run(async () => providersApi.setAddress(p.id, url.trim(), !p.key?.set || (await confirmDialog(W.keepKeyAsk))))}>{W.saveAddress}</button>
          </div>
          <div className={`${hintCls} mt-1`}>{W.addressNote}</div>
        </div>
  );
};
