import { useCallback, useEffect, useRef, useState } from 'react';
import { memory, PAGE, type Filter, type MemoryRow, type Sort, type TabKey } from './memoryApi.mts';

/** What the list is showing: a tier tab, the search text, two filters, an order. */
export interface Query {
  tab: TabKey;
  text: string;
  type: string;
  from: string;
  sort: Sort;
}

export const firstQuery: Query = { tab: 'all', text: '', type: '', from: '', sort: 'newest' };

export const filterOf = (q: Query): Filter => ({ type: q.type || undefined, from: q.from || undefined });

/** Rows for a query, a page at a time. A reply that arrives after a newer
 *  query was asked is dropped, so a slow search cannot overwrite a fast list. */
export function useMemoryRows() {
  const [query, setQuery] = useState<Query>(firstQuery);
  const [rows, setRows] = useState<MemoryRow[]>([]);
  const [hasMore, setHasMore] = useState(false);
  const [status, setStatus] = useState<'loading' | 'ready' | 'error'>('loading');
  const [error, setError] = useState<unknown>(null);
  const [loadingMore, setLoadingMore] = useState(false);
  const asked = useRef(0);

  const reload = useCallback(async () => {
    const mine = ++asked.current;
    setStatus('loading');
    try {
      const f = filterOf(query);
      const got = query.text
        ? await memory.search(query.text, query.tab, f)
        : await memory.list(query.tab, f, query.sort, 0);
      if (mine !== asked.current) return;
      setRows(got);
      // Search is ranked and has no offset; only a list pages.
      setHasMore(!query.text && got.length === PAGE);
      setStatus('ready');
    } catch (e) {
      if (mine !== asked.current) return;
      setError(e);
      setStatus('error');
    }
  }, [query]);

  useEffect(() => { void reload(); }, [reload]);

  const loadMore = useCallback(async () => {
    const mine = asked.current;
    setLoadingMore(true);
    try {
      const got = await memory.list(query.tab, filterOf(query), query.sort, rows.length);
      if (mine !== asked.current) return;
      setRows((cur) => [...cur, ...got.filter((r) => !cur.some((c) => c.id === r.id))]);
      setHasMore(got.length === PAGE);
    } catch (e) {
      if (mine === asked.current) { setError(e); setStatus('error'); }
    } finally {
      setLoadingMore(false);
    }
  }, [query, rows.length]);

  /** A row changed on the server: show its new state where it is. */
  const replaceRow = useCallback((row: MemoryRow) => {
    setRows((cur) => cur.map((r) => (r.id === row.id ? row : r)));
  }, []);

  /** A row was added: show it first when this view would have listed it. */
  const addRow = useCallback((row: MemoryRow) => {
    const fits = !query.text && (query.tab === 'all' || query.tab === row.tier);
    setRows((cur) => {
      if (cur.some((r) => r.id === row.id)) return cur.map((r) => (r.id === row.id ? row : r));
      return fits ? [row, ...cur] : cur;
    });
  }, [query]);

  const dropRows = useCallback((ids: Iterable<string>) => {
    const gone = new Set(ids);
    setRows((cur) => cur.filter((r) => !gone.has(r.id)));
  }, []);

  const change = useCallback((patch: Partial<Query>) => setQuery((q) => ({ ...q, ...patch })), []);

  return { query, change, rows, hasMore, status, error, loadingMore, reload, loadMore, replaceRow, addRow, dropRows };
}
