import React, { useState } from 'react';
import { describeError, memory, ORIGINS, OUTCOMES, TYPES, type MemoryRow, type NewRow, type Tier } from './memoryApi.mts';
import { tierLabel } from './memoryFormat.mts';
import { ModalShell } from './ModalShell';
import { PickField, TextField } from './Fields';
import { ghostBtn, inputCls, primaryBtn } from './memoryStyle';

const TIERS: Tier[] = ['core', 'semantic', 'episodic'];

/** Add one row by hand. A duplicate of an existing row is merged into it by
 *  the store; `onAdded` says which row kept it. */
export const AddRowDialog: React.FC<{
  firstTier: Tier;
  onAdded: (row: MemoryRow, merged: boolean) => void;
  onClose: () => void;
}> = ({ firstTier, onAdded, onClose }) => {
  const [row, setRow] = useState<NewRow>({ content: '', tier: firstTier, type: 'fact', from: 'user' });
  const [busy, setBusy] = useState(false);
  const [failure, setFailure] = useState('');
  const set = (patch: Partial<NewRow>) => setRow((r) => ({ ...r, ...patch }));
  const core = row.tier === 'core';

  const submit = async () => {
    setBusy(true);
    setFailure('');
    try {
      const out = await memory.add({ ...row, content: row.content.trim() });
      onAdded(out.row, out.merged);
    } catch (e) {
      setFailure(describeError(e, 'add the row'));
      setBusy(false);
    }
  };

  return (
    <ModalShell title="Add a row" onClose={onClose}>
      <textarea
        data-no-i18n
        autoFocus
        aria-label="Content"
        placeholder="One or two sentences, found by meaning."
        className={`${inputCls} min-h-24 resize-y`}
        value={row.content}
        onChange={(e) => set({ content: e.target.value })}
      />
      <div className="grid grid-cols-2 gap-3">
        <PickField label="Tier" value={row.tier} options={TIERS} show={tierLabel} onChange={(v) => set({ tier: v as Tier })} />
        <PickField label="Type" value={row.type} options={TYPES} onChange={(v) => set({ type: v })} />
        <PickField label="From" value={row.from} options={ORIGINS} onChange={(v) => set({ from: v })} />
        <PickField label="Outcome" value={row.outcome ?? ''} options={OUTCOMES} none="—" onChange={(v) => set({ outcome: v })} />
      </div>
      <TextField
        label="Scope"
        disabled={core}
        placeholder={core ? 'Core rows are about the person.' : '~/workspace/project, or empty for the person'}
        value={core ? '' : row.scope ?? ''}
        onChange={(v) => set({ scope: v })}
      />
      {failure && <p role="alert" className="text-xs text-red-600 dark:text-red-400">{failure}</p>}
      <div className="flex justify-end gap-2">
        <button className={ghostBtn} onClick={onClose}>Cancel</button>
        <button className={primaryBtn} disabled={busy || !row.content.trim()} onClick={submit}>{busy ? 'Adding…' : 'Add'}</button>
      </div>
    </ModalShell>
  );
};
