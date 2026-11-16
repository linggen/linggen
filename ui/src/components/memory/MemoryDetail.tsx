import React, { useEffect, useMemo, useRef, useState } from 'react';
import { ArrowLeft } from 'lucide-react';
import { confirmDialog } from '../../lib/confirmDialog';
import { describeError, memory, ORIGINS, OUTCOMES, TYPES, type MemoryRow, type Tier } from './memoryApi.mts';
import { draftOf, patchOf, problemWith, SHOWN_FIELDS, type Draft } from './memoryDraft.mts';
import { fmtTime, preview, tierLabel } from './memoryFormat.mts';
import { PickField, TextField } from './Fields';
import { dangerBtn, ghostBtn, inputCls, primaryBtn, sectionCls } from './memoryStyle';

interface Props {
  row: MemoryRow;
  onSaved: (row: MemoryRow) => void;
  onDeleted: (id: string) => void;
  onClose: () => void;
  /** Tells the tab whether there are unsaved edits (to ask before leaving). */
  onDirty: (dirty: boolean) => void;
}

const TIERS: Tier[] = ['core', 'semantic', 'episodic'];

/** One row in full, editable in place. The embedding vector is never here. */
export const MemoryDetail: React.FC<Props> = ({ row, onSaved, onDeleted, onClose, onDirty }) => {
  const [draft, setDraft] = useState<Draft>(() => draftOf(row));
  const [busy, setBusy] = useState(false);
  const [failure, setFailure] = useState('');
  const top = useRef<HTMLElement>(null);
  const patch = useMemo(() => patchOf(row, draft), [row, draft]);
  const problem = problemWith(draft);

  useEffect(() => { setDraft(draftOf(row)); setFailure(''); }, [row]);
  // On a phone the list gives way to the row: show it from its top, not from where the list was scrolled to.
  useEffect(() => {
    if (window.matchMedia('(max-width: 1023px)').matches) top.current?.scrollIntoView({ block: 'start' });
  }, [row.id]);
  useEffect(() => { onDirty(patch !== null); }, [patch, onDirty]);
  useEffect(() => () => onDirty(false), [onDirty]);

  const set = <K extends keyof Draft>(key: K, value: Draft[K]) => setDraft((d) => ({ ...d, [key]: value }));

  const save = async () => {
    if (!patch || problem) return;
    setBusy(true);
    setFailure('');
    try {
      onSaved(await memory.update(row.id, patch));
    } catch (e) {
      setFailure(describeError(e, 'save'));
    } finally {
      setBusy(false);
    }
  };

  const remove = async () => {
    if (!(await confirmDialog(`Delete this row?\n\n“${preview(row.content, 90)}”\n\nThis cannot be undone.`))) return;
    setBusy(true);
    try {
      await memory.remove(row.id);
      onDeleted(row.id);
    } catch (e) {
      setFailure(describeError(e, 'delete'));
      setBusy(false);
    }
  };

  const isCore = draft.tier === 'core';
  return (
    <section ref={top} className={`${sectionCls} space-y-4`} data-testid="memory-detail">
      <div className="flex items-center gap-2">
        <button onClick={onClose} className={`${ghostBtn} lg:hidden`}><ArrowLeft size={14} /> List</button>
        <h2 className="text-xs font-bold uppercase tracking-wider text-slate-700 dark:text-slate-300">{`${tierLabel(row.tier)} row`}</h2>
      </div>

      <textarea
        data-no-i18n
        aria-label="Content"
        className={`${inputCls} min-h-32 resize-y leading-relaxed`}
        value={draft.content}
        onChange={(e) => set('content', e.target.value)}
      />

      <div className="grid grid-cols-1 sm:grid-cols-2 gap-3">
        <PickField label="Tier" value={draft.tier} options={TIERS} show={tierLabel} onChange={(v) => set('tier', v as Tier)} />
        <PickField label="Type" value={draft.type} options={TYPES} onChange={(v) => set('type', v)} />
        <PickField label="From" value={draft.from} options={ORIGINS} onChange={(v) => set('from', v)} />
        <PickField label="Outcome" value={draft.outcome} options={OUTCOMES} none="—" onChange={(v) => set('outcome', v)} />
        <TextField label="Summary" value={draft.summary} hint="One line, shown in the index." onChange={(v) => set('summary', v)} />
        <TextField label="Scope" value={isCore ? '' : draft.scope} disabled={isCore} hint={isCore ? 'Core rows are about the person.' : 'A directory. Empty means the person.'} onChange={(v) => set('scope', v)} />
      </div>

      <label className="flex items-center gap-2 min-h-11 md:min-h-0 text-xs text-slate-600 dark:text-slate-300">
        <input type="checkbox" checked={draft.indexed && !isCore} disabled={isCore} onChange={(e) => set('indexed', e.target.checked)} />
        In its scope's index
      </label>

      {failure && <p role="alert" className="text-xs text-red-600 dark:text-red-400">{failure}</p>}
      {problem && patch && <p className="text-xs text-amber-600 dark:text-amber-400">{problem}</p>}

      <div className="flex items-center gap-2">
        <button className={primaryBtn} disabled={!patch || !!problem || busy} onClick={save}>Save</button>
        <button className={ghostBtn} disabled={!patch || busy} onClick={() => setDraft(draftOf(row))}>Revert</button>
        <span className="flex-1" />
        <button className={dangerBtn} disabled={busy} onClick={remove}>Delete</button>
      </div>

      <Facts row={row} />
    </section>
  );
};

/** What the editor does not offer to change: ids, times, and any other field. */
const Facts: React.FC<{ row: MemoryRow }> = ({ row }) => {
  const fixed: [string, string | null | undefined][] = [
    ['id', row.id], ['Created', fmtTime(row.created_at)], ['Updated', fmtTime(row.updated_at)],
    ['Occurred', fmtTime(row.occurred_at)], ['Host', row.host as string | undefined],
    ['Session', row.source_session as string | undefined],
  ];
  const others = Object.entries(row).filter(([k]) => !SHOWN_FIELDS.has(k));
  return (
    <div className="border-t border-slate-100 dark:border-white/5 pt-3 space-y-1.5">
      <dl className="grid grid-cols-[5.5rem_minmax(0,1fr)] gap-x-3 gap-y-1 text-[11px]">
        {fixed.filter(([, v]) => v).map(([k, v]) => (
          <React.Fragment key={k}>
            <dt className="text-slate-400">{k}</dt>
            <dd data-no-i18n className="text-slate-600 dark:text-slate-300 break-all font-mono">{v}</dd>
          </React.Fragment>
        ))}
      </dl>
      {others.length > 0 && (
        <details className="text-[11px] text-slate-500">
          <summary className="cursor-pointer select-none">All fields</summary>
          <dl className="mt-1 grid grid-cols-[7rem_minmax(0,1fr)] gap-x-3 gap-y-1">
            {others.map(([k, v]) => (
              <React.Fragment key={k}>
                <dt className="text-slate-400">{k}</dt>
                <dd data-no-i18n className="break-all font-mono">{typeof v === 'string' ? v : JSON.stringify(v)}</dd>
              </React.Fragment>
            ))}
          </dl>
        </details>
      )}
    </div>
  );
};
