import React, { useEffect, useRef, useState } from 'react';
import { confirmDialog } from '../../lib/confirmDialog';
import { describeError, isNarrowed, memory, ORIGINS, TYPES, type Filter, type Reach } from './memoryApi.mts';
import { PickField, TextField } from './Fields';
import { ModalShell } from './ModalShell';
import { dangerBtn, ghostBtn } from './memoryStyle';

const REACH: Reach[] = ['all', 'core', 'semantic', 'episodic'];
const REACH_LABEL: Record<Reach, string> = { all: 'Every tier', core: 'Core', semantic: 'Long-term', episodic: 'Episodic' };

/** Forget every row a filter reaches. It counts first and shows the number;
 *  nothing is forgotten without a type, a source or a date. */
export const ForgetDialog: React.FC<{
  first: { reach: Reach; type: string; from: string };
  onForgot: (removed: number) => void;
  onClose: () => void;
}> = ({ first, onForgot, onClose }) => {
  const [reach, setReach] = useState<Reach>(first.reach);
  const [type, setType] = useState(first.type);
  const [from, setFrom] = useState(first.from);
  const [until, setUntil] = useState('');
  const [count, setCount] = useState<number | null>(null);
  const [busy, setBusy] = useState(false);
  const [failure, setFailure] = useState('');
  const asked = useRef(0);

  const filter: Filter = { type: type || undefined, from: from || undefined, until: until || undefined };
  const narrowed = isNarrowed(filter);

  useEffect(() => {
    const mine = ++asked.current;
    setCount(null);
    if (!narrowed) return;
    const timer = setTimeout(() => {
      memory.count(reach, { type: type || undefined, from: from || undefined, until: until || undefined }).then(
        (n) => { if (mine === asked.current) setCount(n); },
        (e) => { if (mine === asked.current) setFailure(describeError(e, 'count the rows')); },
      );
    }, 250);
    return () => clearTimeout(timer);
  }, [reach, type, from, until, narrowed]);

  const forget = async () => {
    if (!count) return;
    const ok = await confirmDialog(`Forget ${count} row${count === 1 ? '' : 's'}? This cannot be undone.`);
    if (!ok) return;
    setBusy(true);
    setFailure('');
    try {
      onForgot(await memory.forget(reach, filter));
    } catch (e) {
      setFailure(describeError(e, 'forget the rows'));
      setBusy(false);
    }
  };

  return (
    <ModalShell title="Forget by filter" onClose={onClose}>
      <div className="grid grid-cols-2 gap-3">
        <PickField label="Where" value={reach} options={REACH} show={(v) => REACH_LABEL[v as Reach]} onChange={(v) => setReach(v as Reach)} />
        <PickField label="Type" value={type} options={TYPES} none="Any" onChange={setType} />
        <PickField label="From" value={from} options={ORIGINS} none="Any" onChange={setFrom} />
        <TextField label="Older than" value={until} placeholder="YYYY-MM-DD" onChange={setUntil} />
      </div>
      <p className="text-xs text-slate-500 dark:text-slate-400" data-testid="forget-count">
        {!narrowed ? 'Pick a type, a source or a date.' : count === null ? 'Counting…' : `${count} row${count === 1 ? '' : 's'} match.`}
      </p>
      {failure && <p role="alert" className="text-xs text-red-600 dark:text-red-400">{failure}</p>}
      <div className="flex justify-end gap-2">
        <button className={ghostBtn} onClick={onClose}>Cancel</button>
        <button className={dangerBtn} disabled={busy || !count} onClick={forget}>{count ? `Forget ${count} row${count === 1 ? '' : 's'}` : 'Forget'}</button>
      </div>
    </ModalShell>
  );
};
