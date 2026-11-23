/**
 * One provider the person pays with a key. Under its head: the key (pasted
 * once; the page only ever learns that it is set), the families to tick, models
 * added by name, the live-voice lines, the price date, and a folded
 * 「Advanced」. Every action calls its route (lib/endpoints `providersApi`).
 */
import React, { useState } from 'react';
import { providersApi } from '../../lib/endpoints';
import { W, say } from '../../lib/modelsWords.mts';
import { addedByName, liveModels, type Provider } from '../../lib/providers.mts';
import { FamilyList } from './FamilyList';
import { RowMenu, Row, Tag, ghostBtn, hintCls, inputCls, primaryBtn } from './shell';
import type { Run } from './useProviders';

type Props = {
  provider: Provider;
  open: boolean;
  onToggle: () => void;
  run: Run;
  busy: boolean;
  makeDefault: (modelId: string) => Promise<void>;
};

/** The head's second line: what is ticked, or what is missing. */
function summary(p: Provider): string {
  if (!p.key?.set) return W.noKeyYet;
  const names = (p.families?.families ?? []).filter((f) => f.ticked && f.current).map((f) => f.current!.name);
  return names.length ? names.join(' · ') : W.nothingTicked;
}

export const KeyRow: React.FC<Props> = (props) => {
  const { provider: p, open, onToggle } = props;
  return (
    <Row testId={p.provider} name={p.label} tag={<Tag tone="blue">{W.kindKey}</Tag>} subtitle={summary(p)} open={open} onToggle={onToggle}>
      <KeyField {...props} />
      <Models {...props} />
      <Advanced {...props} />
    </Row>
  );
};

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
  const set = !!p.key?.set;
  return (
    <div className="flex flex-wrap items-center gap-2">
      <span className="w-12 shrink-0 text-[11px] font-bold text-slate-500">{W.keyLabel}</span>
      {set && !replacing ? (
        <>
          <span data-testid="key-set" className={`${inputCls} font-mono`}>{W.keySet}</span>
          <button type="button" className={ghostBtn} disabled={busy} onClick={() => setReplacing(true)}>{W.replaceKey}</button>
          <button type="button" className={`${ghostBtn} !text-red-600`} disabled={busy} onClick={() => void run(() => providersApi.setKey(p.id, null))}>{W.removeKey}</button>
          <span className={hintCls}>{W.shareKey}</span>
        </>
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

/** The models: the families, what was added by name, the live-voice lines, the price date. */
const Models: React.FC<Props> = (props) => {
  const { provider: p, run, busy, makeDefault } = props;
  const typed = addedByName(p);
  const live = liveModels(p);
  const catalog = p.families?.catalog;
  const none = !p.families?.families.length && !typed.length && !live.length;
  return (
    <div className="flex flex-wrap gap-2 sm:flex-nowrap">
      <span className="w-12 shrink-0 text-[11px] font-bold text-slate-500">{W.models}</span>
      <div className="min-w-0 flex-1">
        {none && <p className={hintCls}>{W.noModelsYet}</p>}
        <FamilyList {...props} />
        {typed.length > 0 && (
          <div data-testid="typed-models" className="mt-2">
            <div className="text-[11px] font-bold text-slate-400">{W.addedByName}</div>
            {typed.map((m) => (
              <div key={m.id} className="flex items-center gap-2 py-1 text-xs" data-testid="typed-model">
                <span className="min-w-0 flex-1 truncate font-semibold" data-no-i18n>{m.model}</span>
                <RowMenu disabled={busy} items={[
                  { label: W.makeDefault, run: () => void makeDefault(m.id) },
                  { label: W.removeModel, run: () => void run(() => providersApi.removeModel(p.id, m.id)) },
                ]} />
              </div>
            ))}
          </div>
        )}
        {live.map((m) => (
          <div key={m.id} className="mt-1 flex flex-wrap items-center gap-1.5 text-xs" data-testid="live-model">
            <span className="font-semibold" data-no-i18n>{m.model}</span>
            <Tag>{W.liveTag}</Tag>
            <span className={hintCls}>{m.ready ? W.ready : W.notReady}</span>
          </div>
        ))}
        {catalog && p.families!.families.length > 0 && (
          <p data-testid="price-date" className={`${hintCls} mt-2`}>
            {say(W.estimatesFrom, { date: catalog.observed_at })}
            {catalog.stale && <> {say(W.listOld, { n: catalog.age_days })}</>}
          </p>
        )}
      </div>
    </div>
  );
};

/** Folded: a model the list does not have, and the address. */
const Advanced: React.FC<Props> = ({ provider: p, run, busy }) => {
  const [name, setName] = useState('');
  const [url, setUrl] = useState(p.url);
  const add = async () => {
    if (!name.trim()) return;
    if (await run(() => providersApi.addModel(p.id, name.trim()))) setName('');
  };
  return (
    <details data-testid="advanced">
      <summary className="cursor-pointer text-[11px] text-slate-500">{W.advanced}</summary>
      <div className="mt-2 space-y-3">
        <div>
          <div className={hintCls}>{W.notInList}</div>
          <div className="mt-1 flex flex-wrap items-center gap-2">
            <input className={`${inputCls} w-64 max-w-full flex-1`} placeholder={W.modelName} aria-label={W.modelName} value={name} onChange={(e) => setName(e.target.value)} onKeyDown={(e) => { if (e.key === 'Enter') void add(); }} />
            <button type="button" className={ghostBtn} disabled={busy || !name.trim()} onClick={() => void add()}>{W.add}</button>
          </div>
        </div>
        <div>
          <div className={hintCls}>{W.address}</div>
          <div className="mt-1 flex flex-wrap items-center gap-2">
            <input className={`${inputCls} w-64 max-w-full flex-1 font-mono`} aria-label={W.address} value={url} onChange={(e) => setUrl(e.target.value)} />
            <button type="button" className={ghostBtn} disabled={busy || !url.trim() || url.trim() === p.url} onClick={() => void run(() => providersApi.setAddress(p.id, url.trim()))}>{W.saveAddress}</button>
          </div>
          <div className={`${hintCls} mt-1`}>{W.addressNote}</div>
        </div>
      </div>
    </details>
  );
};
