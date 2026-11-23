/**
 * The families of one key provider: a checkbox each, what it runs now and its
 * dated estimated price, the move notice, and ⋯. Everything shown is the
 * engine's answer (`Provider.families`); a click calls the family route and
 * the list is redrawn from what it returns.
 */
import React, { useState } from 'react';
import { providersApi } from '../../lib/endpoints';
import { W, say } from '../../lib/modelsWords.mts';
import { dollars, familyModel, searchFamilies, splitFamilies, usedWords, type Family, type FamilyAction, type PriceLine, type Provider } from '../../lib/providers.mts';
import { RowMenu, Tag, hintCls, inputCls, linkBtn } from './shell';
import type { Run } from './useProviders';

/** More folded families than this get a search box. */
const SEARCH_FROM = 8;
/** Folded families drawn at once: a list of hundreds is searched, not scrolled. */
const DRAWN = 50;

type Props = { provider: Provider; run: Run; busy: boolean; makeDefault: (modelId: string) => Promise<void> };

export const FamilyList: React.FC<Props> = (props) => {
  const { provider } = props;
  const view = provider.families;
  const { shown, folded } = splitFamilies(view);
  const [query, setQuery] = useState('');
  if (!view) return null;
  const matches = searchFamilies(folded, query);
  return (
    <div data-testid="family-list">
      {shown.map((f) => <FamilyRow key={f.family} family={f} {...props} />)}
      {folded.length > 0 && (
        <details className="mt-1" data-testid="family-more">
          <summary className="cursor-pointer py-1 text-[11px] text-slate-500">{say(W.moreN, { n: folded.length })}</summary>
          {folded.length > SEARCH_FROM && (
            <div className="my-1.5 flex flex-wrap items-center gap-2">
              <input className={`${inputCls} w-56 max-w-full`} placeholder={W.searchModels} aria-label={W.searchModels} value={query} onChange={(e) => setQuery(e.target.value)} />
              <span className={hintCls}>{say(W.totalN, { n: view.families.length })}</span>
            </div>
          )}
          {matches.slice(0, DRAWN).map((f) => <FamilyRow key={f.family} family={f} {...props} />)}
          {matches.length > DRAWN && <p className={hintCls}>{W.searchToSee}</p>}
        </details>
      )}
      <p className={`${hintCls} mt-2`}>{W.followNote}</p>
    </div>
  );
};

/** The estimated price, when the engine has one: input and output per million. */
const Price: React.FC<{ price?: PriceLine }> = ({ price }) => {
  if (!price) return <span>{W.noPrice}</span>;
  return (
    <>
      <span>{say(W.priceIn, { input: dollars(price.input), output: dollars(price.output) })}</span>
      {price.source === 'catalog' && <span className="ml-1 text-[10px] font-semibold text-slate-400">{W.estimate}</span>}
    </>
  );
};

const FamilyRow: React.FC<Props & { family: Family }> = ({ provider, family: f, run, busy, makeDefault }) => {
  const act = (action: FamilyAction) => run(() => providersApi.family(provider.id, f.family, action));
  const model = familyModel(f);
  const menu = f.ticked && f.row ? [
    ...(f.mode === 'pinned'
      ? [{ label: W.followAgain, run: () => void act({ action: 'follow' }) }]
      : [{ label: W.pinVersion, run: () => void act({ action: 'pin' }) }]),
    { label: W.makeDefault, run: () => void makeDefault(f.row!) },
  ] : [];
  return (
    <div data-testid="family-row" data-family={f.family} className="border-b border-slate-100 py-2 last:border-b-0 dark:border-white/5">
      <div className="flex items-start gap-2">
        <input
          type="checkbox"
          className="mt-0.5 h-3.5 w-3.5 shrink-0 accent-blue-600"
          aria-label={f.label}
          checked={f.ticked}
          disabled={busy}
          onChange={() => void act(f.ticked ? { action: 'untick' } : { action: 'tick' })}
        />
        <div className="min-w-0 flex-1">
          <div className="flex flex-wrap items-center gap-1.5 text-xs font-semibold">
            <span data-no-i18n>{f.label}</span>
            {f.mode === 'pinned' && <Tag tone="blue">{W.pinned}</Tag>}
            {f.references.map((r) => {
              const used = usedWords(r);
              return <Tag key={`${r.place}${r.who ?? ''}`} tone="green">{used.who ? say(used.word, { who: used.who }) : used.word}</Tag>;
            })}
          </div>
          <div className="mt-0.5 flex flex-wrap gap-x-1.5 text-[11px] text-slate-500 dark:text-slate-400">
            {model ? (
              <>
                <span>{say(f.ticked ? W.now : W.wouldBe, { name: model.name })}</span>
                <span aria-hidden>·</span>
                <span><Price price={model.price} /></span>
              </>
            ) : (
              <span>{f.hold === 'no_access' ? W.failNoAccess : W.holdChoose}</span>
            )}
          </div>
          {f.choose.some((c) => c.chooseable) && (
            <select className={`${inputCls} mt-1.5 max-w-full`} aria-label={W.chooseVersion} value="" disabled={busy} onChange={(e) => e.target.value && void act({ action: 'choose', model: e.target.value })}>
              <option value="">{W.chooseVersion}</option>
              {f.choose.filter((c) => c.chooseable).map((c) => <option key={c.model} value={c.model}>{c.name}</option>)}
            </select>
          )}
        </div>
        {menu.length > 0 && <RowMenu items={menu} disabled={busy} />}
      </div>
      {f.notice && <MoveNotice family={f} act={act} busy={busy} />}
    </div>
  );
};

/** One move, until it is answered: 知道了, 换回, or 固定在这个版本. */
const MoveNotice: React.FC<{ family: Family; act: (a: FamilyAction) => Promise<boolean>; busy: boolean }> = ({ family: f, act, busy }) => {
  const n = f.notice!;
  const move = n.move;
  return (
    <div data-testid="move-notice" className="mt-2 rounded-lg bg-amber-50 p-2.5 text-xs leading-relaxed text-slate-700 dark:bg-amber-500/10 dark:text-slate-200 md:ml-5">
      <span>{say(n.kind === 'followed' ? W.movedLine : W.wentBackLine, { from: n.from, to: n.to })}</span>
      <span className="mt-1 flex flex-wrap gap-3">
        <button type="button" disabled={busy} className={linkBtn} onClick={() => void act({ action: 'ack', move })}>{W.gotIt}</button>
        {n.kind === 'followed' && n.can_go_back && (
          <button type="button" disabled={busy} className={linkBtn} onClick={() => void act({ action: 'back', move })}>{W.switchBack}</button>
        )}
        {n.kind === 'followed' && f.mode === 'follow' && (
          <button type="button" disabled={busy} className={linkBtn} onClick={() => void act({ action: 'pin', move })}>{W.pinVersion}</button>
        )}
      </span>
    </div>
  );
};
