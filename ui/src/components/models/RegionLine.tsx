/**
 * 「地区：国际 · 改」 above the providers: the edition the engine offers, and
 * the owner's choice behind it (自动 or one of the two). The page decides
 * nothing: the pick is sent to the engine and the list is redrawn from its
 * answer. A region changes what is offered for a new setup, never what is set up.
 */
import React, { useState } from 'react';
import { providersApi } from '../../lib/endpoints';
import { W } from '../../lib/modelsWords.mts';
import { editionWord, type EditionChoice, type ProvidersAnswer } from '../../lib/providers.mts';
import { hintCls, inputCls, linkBtn } from './shell';
import type { Run } from './useProviders';

const CHOICES: { choice: EditionChoice; word: string }[] = [
  { choice: 'auto', word: W.automatic },
  { choice: 'international', word: W.regionInternational },
  { choice: 'mainland', word: W.regionMainland },
];

export const RegionLine: React.FC<{ edition: ProvidersAnswer['edition']; run: Run; busy: boolean }> = ({ edition, run, busy }) => {
  const [open, setOpen] = useState(false);
  return (
    <div className="mt-2" data-testid="region">
      <p className={`${hintCls} flex flex-wrap items-center gap-1.5`}>
        <span data-testid="region-line"><span>{W.region}</span> <span>{editionWord(edition.edition)}</span></span>
        <span aria-hidden>·</span>
        <button type="button" data-testid="region-change" className={linkBtn} onClick={() => setOpen(!open)}>{W.regionChange}</button>
      </p>
      {open && (
        <div className="mt-1.5 flex flex-wrap items-center gap-2">
          <select data-testid="region-choice" className={`${inputCls} px-2 py-1 dark:[color-scheme:dark]`} aria-label={W.regionChange} value={edition.choice} disabled={busy}
            onChange={(e) => void run(() => providersApi.setEdition(e.target.value as EditionChoice)).then(() => setOpen(false))}>
            {CHOICES.map(({ choice, word }) => <option key={choice} value={choice}>{word}</option>)}
          </select>
          <span className={hintCls}>{W.regionNote}</span>
        </div>
      )}
    </div>
  );
};
