import React from 'react';
import { describeError, MemoryError, type MemoryRow } from './memoryApi.mts';
import { preview, relAge, shortPath, tierLabel } from './memoryFormat.mts';
import { chipCls, tierChipCls } from './memoryStyle';

interface Props {
  rows: MemoryRow[];
  status: 'loading' | 'ready' | 'error';
  error: unknown;
  /** Why the list is empty, in words: the search, the filters, or nothing. */
  emptyWhy: string;
  selectedId: string | null;
  checked: Set<string>;
  hasMore: boolean;
  loadingMore: boolean;
  onSelect: (id: string) => void;
  onCheck: (id: string) => void;
  onLoadMore: () => void;
  onRetry: () => void;
}

const note = 'rounded-xl border border-dashed border-slate-200 dark:border-white/10 p-6 text-center text-xs text-slate-500 dark:text-slate-400';

/** The rows, or one plain line saying why there are none. */
export const MemoryList: React.FC<Props> = (p) => {
  if (p.status === 'error') return <ErrorNote error={p.error} onRetry={p.onRetry} />;
  if (p.status === 'loading' && p.rows.length === 0) return <div className={note}>Loading…</div>;
  if (p.rows.length === 0) return <div className={note} data-testid="memory-empty">{p.emptyWhy}</div>;
  return (
    <div className="space-y-1.5" role="listbox" aria-label="Memory rows">
      {p.rows.map((r) => (
        <Row key={r.id} row={r} selected={r.id === p.selectedId} checked={p.checked.has(r.id)} onSelect={p.onSelect} onCheck={p.onCheck} />
      ))}
      {p.hasMore && (
        <button onClick={p.onLoadMore} disabled={p.loadingMore} className="w-full py-2 text-xs font-semibold text-blue-600 dark:text-blue-400 hover:underline disabled:opacity-50">
          {p.loadingMore ? 'Loading…' : 'Load more'}
        </button>
      )}
    </div>
  );
};

const ErrorNote: React.FC<{ error: unknown; onRetry: () => void }> = ({ error, onRetry }) => {
  const calm = error instanceof MemoryError && error.forbidden;
  return (
    <div className={note} data-testid="memory-error">
      <p>{describeError(error, 'read memory')}</p>
      {!calm && <button onClick={onRetry} className="mt-2 font-semibold text-blue-600 dark:text-blue-400 hover:underline">Try again</button>}
    </div>
  );
};

const Row: React.FC<{
  row: MemoryRow; selected: boolean; checked: boolean;
  onSelect: (id: string) => void; onCheck: (id: string) => void;
}> = ({ row, selected, checked, onSelect, onCheck }) => (
  <div
    role="option"
    aria-selected={selected}
    data-testid="memory-row"
    onClick={() => onSelect(row.id)}
    className={`flex gap-2.5 items-start rounded-lg border px-3 py-2 cursor-pointer transition-colors ${
      selected
        ? 'border-blue-500 bg-blue-50 dark:bg-blue-500/15'
        : 'border-slate-200 dark:border-white/5 bg-white dark:bg-[#141414] hover:bg-slate-50 dark:hover:bg-white/[0.03]'
    }`}
  >
    <input
      type="checkbox"
      aria-label="Select row"
      className="mt-0.5 shrink-0"
      checked={checked}
      onClick={(e) => e.stopPropagation()}
      onChange={() => onCheck(row.id)}
    />
    <div className="min-w-0 flex-1">
      <p data-no-i18n className="text-xs text-slate-800 dark:text-slate-200 break-words line-clamp-3">{preview(row.content)}</p>
      <div className="mt-1 flex flex-wrap items-center gap-1.5">
        <span className={tierChipCls(row.tier)}>{tierLabel(row.tier)}</span>
        <span className={chipCls}>{row.type}</span>
        <span className={chipCls}>{row.from}</span>
        {row.indexed && <span className={chipCls}>indexed</span>}
        {row.scope && <span data-no-i18n className={chipCls} title={row.scope}>{shortPath(row.scope)}</span>}
        {typeof row.hybrid_score === 'number' && <span className={chipCls} title="Relevance">{row.hybrid_score.toFixed(2)}</span>}
        <span className="text-[10px] text-slate-400 ml-auto">{relAge(row.updated_at ?? row.created_at)}</span>
      </div>
    </div>
  </div>
);
