import React, { useCallback, useEffect, useRef, useState } from 'react';
import { confirmDialog } from '../../lib/confirmDialog';
import { AddRowDialog } from './AddRowDialog';
import { ForgetDialog } from './ForgetDialog';
import { MemoryDetail } from './MemoryDetail';
import { MemoryDays } from './MemoryDays';
import { MemoryReview } from './MemoryReview';
import { MemoryList } from './MemoryList';
import { MemoryStats } from './MemoryStats';
import { MemoryToolbar } from './MemoryToolbar';
import { describeError, memory, type Reach, type Tier } from './memoryApi.mts';
import { tierLabel } from './memoryFormat.mts';
import { filterOf, useMemoryRows, type Query } from './useMemoryRows';

type Dialog = 'add' | 'forget' | null;

/** Why a list is empty, in a few words. */
function emptyWhy(q: Query): string {
  if (q.text) return `Nothing matches “${q.text}”.`;
  if (q.type || q.from) return 'Nothing matches these filters.';
  return q.tab === 'all' ? 'Memory is empty.' : `No ${tierLabel(q.tab).toLowerCase()} rows yet.`;
}

/** Settings → Memory: the store's rows, browsed and edited through the
 *  engine's own door (`POST /api/memory/<verb>`). */
export const MemoryTab: React.FC = () => {
  const rows = useMemoryRows();
  const { query, change } = rows;
  const [reviewCount, setReviewCount] = useState(0);
  const [subview, setSubview] = useState<'Rows' | 'Days' | 'Review'>('Rows');
  const [selectedId, setSelectedId] = useState<string | null>(null);
  const [checked, setChecked] = useState<Set<string>>(new Set());
  const [dialog, setDialog] = useState<Dialog>(null);
  const [notice, setNotice] = useState('');
  const [statsVersion, setStatsVersion] = useState(0);
  const dirty = useRef(false);
  const onDirty = useCallback((d: boolean) => { dirty.current = d; }, []);

  const selected = rows.rows.find((r) => r.id === selectedId) ?? null;
  const changed = () => setStatsVersion((v) => v + 1);

  useEffect(() => { void memory.issues().then((q) => setReviewCount(q.open_count)).catch(() => {}); }, []);

  // Ticks follow the rows that are still listed.
  useEffect(() => {
    const ids = new Set(rows.rows.map((r) => r.id));
    setChecked((cur) => (cur.size && [...cur].some((id) => !ids.has(id)) ? new Set([...cur].filter((id) => ids.has(id))) : cur));
  }, [rows.rows]);

  /** Leaving a row with unsaved edits asks first, in the app's own dialog. */
  const mayLeave = async () => !dirty.current || confirmDialog('Discard unsaved changes?');

  const select = async (id: string) => {
    if (id !== selectedId && !(await mayLeave())) return;
    setSelectedId(id);
  };
  const close = async () => {
    if (await mayLeave()) setSelectedId(null);
  };
  const ask = async (patch: Partial<Query>) => {
    if (!(await mayLeave())) return;
    setNotice('');
    change(patch);
  };
  const toggle = (id: string) => setChecked((cur) => {
    const next = new Set(cur);
    if (!next.delete(id)) next.add(id);
    return next;
  });

  const deleteChecked = async () => {
    const ids = [...checked];
    if (!(await confirmDialog(`Delete ${ids.length} row${ids.length === 1 ? '' : 's'}? This cannot be undone.`))) return;
    const gone: string[] = [];
    let failure = '';
    for (const id of ids) {
      try { await memory.remove(id); gone.push(id); } catch (e) { failure = describeError(e, 'delete a row'); }
    }
    rows.dropRows(gone);
    changed();
    setNotice(failure || `Deleted ${gone.length} row${gone.length === 1 ? '' : 's'}.`);
  };

  const forgot = (removed: number) => {
    setDialog(null);
    setNotice(`Forgot ${removed} row${removed === 1 ? '' : 's'}.`);
    void rows.reload();
    changed();
  };

  return (
    <div className="space-y-4">
      <MemoryStats version={statsVersion} />

      <div className="flex gap-1">
        {(['Rows', 'Days', 'Review'] as const).map((view) => (
          <button key={view} className={`px-3 py-1.5 rounded-lg text-xs font-bold ${subview === view ? 'bg-blue-600 text-white' : 'text-slate-600 dark:text-slate-300 hover:bg-slate-100 dark:hover:bg-white/5'}`} onClick={async () => { if (await mayLeave()) setSubview(view); }}>
            {view}{view === 'Review' && reviewCount > 0 && ` (${reviewCount})`}
          </button>
        ))}
      </div>
      {/* SUBVIEW MOUNT */}
      {subview === 'Days' && <MemoryDays />}
      {subview === 'Review' && <MemoryReview onCount={setReviewCount} />}
      {subview === 'Rows' && <div data-testid="memory-subview-rows" className="space-y-3">
        <MemoryToolbar
          query={query}
          checkedCount={checked.size}
          onChange={ask}
          onAdd={() => setDialog('add')}
          onForget={() => setDialog('forget')}
          onDeleteChecked={deleteChecked}
          onClearChecked={() => setChecked(new Set())}
        />
        {notice && <p role="status" className="text-xs text-slate-500 dark:text-slate-400">{notice}</p>}
        <div className="grid grid-cols-1 lg:grid-cols-[minmax(0,1fr)_minmax(0,26rem)] gap-4 items-start">
          <div className={selected ? 'hidden lg:block' : undefined}>
            <MemoryList
              rows={rows.rows}
              status={rows.status}
              error={rows.error}
              emptyWhy={emptyWhy(query)}
              selectedId={selectedId}
              checked={checked}
              hasMore={rows.hasMore}
              loadingMore={rows.loadingMore}
              onSelect={select}
              onCheck={toggle}
              onLoadMore={rows.loadMore}
              onRetry={rows.reload}
            />
          </div>
          {selected ? (
            <div className="lg:sticky lg:top-0">
              <MemoryDetail
                row={selected}
                onSaved={(row) => { rows.replaceRow(row); setNotice(''); changed(); }}
                onDeleted={(id) => { rows.dropRows([id]); setSelectedId(null); setNotice(''); changed(); }}
                onClose={close}
                onDirty={onDirty}
              />
            </div>
          ) : (
            <p className="hidden lg:block text-xs text-slate-400 pt-2">Select a row to read or edit it.</p>
          )}
        </div>
      </div>}

      {dialog === 'add' && (
        <AddRowDialog
          firstTier={query.tab === 'all' ? 'semantic' : (query.tab as Tier)}
          onClose={() => setDialog(null)}
          onAdded={(row, merged) => {
            setDialog(null);
            rows.addRow(row);
            setSelectedId(row.id);
            setNotice(merged ? 'That was already in memory; the existing row was kept.' : 'Row added.');
            changed();
          }}
        />
      )}
      {dialog === 'forget' && (
        <ForgetDialog
          first={{ reach: (query.tab === 'all' ? 'all' : query.tab) as Reach, ...forgetFilter(query) }}
          onClose={() => setDialog(null)}
          onForgot={forgot}
        />
      )}
    </div>
  );
};

const forgetFilter = (q: Query) => ({ type: filterOf(q).type ?? '', from: filterOf(q).from ?? '' });
