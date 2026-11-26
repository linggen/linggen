/**
 * One provider the person pays with a key. Under its head: the key (pasted
 * once; the page only ever learns that it is set), the families to tick, models
 * added by name, the live-voice lines, the price date, and a folded
 * 「Advanced」. Every action calls its route (lib/endpoints `providersApi`).
 */
import React, { useState } from 'react';
import { providersApi } from '../../lib/endpoints';
import { W, say } from '../../lib/modelsWords.mts';
import { addedByName, capShare, liveModels, spendMeter, type Provider } from '../../lib/providers.mts';
import { FamilyList } from './FamilyList';
import { RowMenu, Row, Tag, ghostBtn, hintCls, inputCls, primaryBtn, sectionLabel, sectionRule } from './shell';
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
    <Row testId={p.provider} name={p.label} way={p.way} subtitle={summary(p)} open={open} onToggle={onToggle}
      tags={<>{p.retained && <span title={W.keptNote} data-testid="retained"><Tag tone="amber">{W.kept}</Tag></span>}{p.used_by.includes('chat') && <Tag tone="blue">{W.chat}</Tag>}{p.used_by.includes('live_voice') && <Tag tone="blue">{W.liveVoice}</Tag>}</>}
      detail={p.key?.set ? <span>{say(W.billedBy, { provider: p.label })}</span> : <span className="rounded-md border border-slate-200 bg-white px-2 py-0.5 font-semibold dark:border-white/10 dark:bg-white/5">{W.fillKey}</span>}>
      <KeyField {...props} />
      <Models {...props} />
      <SpendCap {...props} />
      {!p.listed && <Address {...props} />}
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
        <>
          <span data-testid="key-set" className={`${inputCls} font-mono`}>{W.keySet}</span>
          <button type="button" className={ghostBtn} disabled={busy} onClick={() => setReplacing(true)}>{W.replaceKey}</button>
          <button type="button" className={`${ghostBtn} !text-red-600`} disabled={busy} onClick={() => void run(() => providersApi.setKey(p.id, null))}>{W.removeKey}</button>
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
  const { provider: p } = props;
  const typed = addedByName(p);
  const live = liveModels(p);
  const catalog = p.families?.catalog;
  const none = !p.families?.families.length && !typed.length && !live.length;
  return (
    <div className={`${sectionRule} grid gap-2 sm:grid-cols-[4rem_minmax(0,1fr)]`}>
      <span className={sectionLabel}>{W.models}</span>
      <div className="min-w-0 flex-1">
        {none && <p className={hintCls}>{W.noModelsYet}</p>}
        <FamilyList {...props} />

        {live.map((m) => (
          <div key={m.id} className="mt-1 flex flex-wrap items-center gap-1.5 text-xs" data-testid="live-model">
            <span className="font-semibold" data-no-i18n>{m.model}</span>
            <Tag>{W.liveTag}</Tag>
            <span className={hintCls}>{m.ready ? W.liveAvailable : W.liveMissingKey}</span>
            <span className={hintCls}>{W.lingyuUses}</span>
          </div>
        ))}
        <CustomModels {...props} />
        {catalog && !p.families?.rates_unavailable && p.families!.families.length > 0 && (
          <p data-testid="price-date" className={`${hintCls} mt-2`}>
            {say(W.estimatesFrom, { date: catalog.observed_at })}
            {catalog.stale && <> {say(W.listOld, { n: catalog.age_days })}</>}
          </p>
        )}
      </div>
    </div>
  );
};

const CustomModels: React.FC<Props> = ({ provider: p, run, busy, makeDefault }) => {
  const [name, setName] = useState('');
  const add = async () => {
    if (!name.trim()) return;
    if (await run(() => providersApi.addModel(p.id, name.trim()))) setName('');
  };
  return (
    <div data-testid="custom-models" className="mt-2">
          <div className={hintCls}>{W.custom}</div>
          {addedByName(p).map((m) => (
            <div key={m.id} className="flex items-center gap-2 py-1 text-xs" data-testid="typed-model">
              <span className="min-w-0 flex-1 truncate font-semibold" data-no-i18n>{m.model}</span>
              <RowMenu disabled={busy} items={[
                { label: W.makeDefault, run: () => void makeDefault(m.id) },
                { label: W.removeModel, run: () => void run(() => providersApi.removeModel(p.id, m.id)) },
              ]} />
            </div>
          ))}
          <div className="mt-1 flex flex-wrap items-center gap-2">
            <input className={`${inputCls} w-64 max-w-full flex-1`} placeholder={W.modelName} aria-label={W.modelName} value={name} onChange={(e) => setName(e.target.value)} onKeyDown={(e) => { if (e.key === 'Enter') void add(); }} />
            <button type="button" className={ghostBtn} disabled={busy || !name.trim()} onClick={() => void add()}>{W.add}</button>
          </div>
    </div>
  );
};

/** One field 「Monthly cap $」 and the meter, under the models. */
const SpendCap: React.FC<Props> = ({ provider: p, run, busy }) => {
  const [text, setText] = useState('');
  if (!p.spend) return null;
  const usd = Number(text);
  const ok = text.trim() !== '' && Number.isFinite(usd) && usd > 0;
  return (
    <div data-testid="spend-cap" className={`${sectionRule} sm:pl-[4.5rem]`}>
      <div className="flex flex-wrap items-center gap-2">
        <span className={hintCls}>{W.capLabel}</span>
        <input className={`${inputCls} w-24`} inputMode="decimal" aria-label={W.capLabel} placeholder={p.spend.cap_usd === undefined ? '200' : String(p.spend.cap_usd)} value={text} onChange={(e) => setText(e.target.value)} />
        <button type="button" className={ghostBtn} disabled={busy || !ok} onClick={() => void run(() => providersApi.setCap(p.id, usd)).then((done) => done && setText(''))}>{W.setCap}</button>
        {p.spend.cap_usd !== undefined && <button type="button" className={ghostBtn} disabled={busy} onClick={() => void run(() => providersApi.setCap(p.id, null))}>{W.clearCap}</button>}
      </div>
      <div className={`${hintCls} mt-1`} data-testid="spend-meter" title={say(W.spendNote, { date: p.spend.prices_as_of })}>{spendMeter(p.spend)}</div>
      {p.spend.cap_usd !== undefined && <div className="mt-1 h-1 w-48 overflow-hidden rounded bg-slate-200 dark:bg-white/10"><div className={`h-full ${capShare(p.spend) >= 1 ? 'bg-red-500' : capShare(p.spend) >= 0.8 ? 'bg-amber-500' : 'bg-emerald-500'}`} style={{ width: `${capShare(p.spend) * 100}%` }} /></div>}
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
            <button type="button" className={ghostBtn} disabled={busy || !url.trim() || url.trim() === p.url} onClick={() => void run(() => providersApi.setAddress(p.id, url.trim(), !p.key?.set || window.confirm(W.keepKeyAsk)))}>{W.saveAddress}</button>
          </div>
          <div className={`${hintCls} mt-1`}>{W.addressNote}</div>
        </div>
  );
};
