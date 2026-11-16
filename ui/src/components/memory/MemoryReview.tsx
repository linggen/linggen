import React, { useCallback, useEffect, useRef, useState } from 'react';
import { startSolve } from './dreamApi.mts';
import { OpenRun } from './OpenRun';
import { confirmDialog } from '../../lib/confirmDialog';
import { describeError, memory, MemoryError, type MemoryRow, type ReviewIssue } from './memoryApi.mts';
import { fmtTime } from './memoryFormat.mts';
import { chipCls, dangerBtn, ghostBtn, primaryBtn, sectionCls } from './memoryStyle';

export const MemoryReview: React.FC<{ onCount: (count: number) => void }> = ({ onCount }) => {
  const [items, setItems] = useState<ReviewIssue[] | null>(null);
  const [error, setError] = useState<unknown>(null);
  const [opened, setOpened] = useState<Record<string, MemoryRow | null>>({});
  const [count, setCount] = useState(0);
  const [busy, setBusy] = useState('');
  const [solving, setSolving] = useState(false);
  const [notice, setNotice] = useState('');
  const [run, setRun] = useState<string | null>(null);
  const mounted = useRef(false);
  const timer = useRef<ReturnType<typeof setInterval> | null>(null);
  const stop = useCallback(() => { if (timer.current) clearInterval(timer.current); timer.current = null; }, []);
  const load = useCallback(async () => {
    try { const q = await memory.issues(); setItems(q.issues); setCount(q.open_count); onCount(q.open_count); setError(null); }
    catch (e) { setError(e); }
  }, [onCount]);
  useEffect(() => { mounted.current = true; void load(); return () => { mounted.current = false; stop(); }; }, [load, stop]);
  const solve = async () => {
    setSolving(true); setNotice(''); setRun(null);
    try {
      const session = await startSolve();
      if (!mounted.current) return;
      setRun(session); setNotice('Solve started.'); void load(); stop();
      const until = Date.now() + 180_000;
      timer.current = setInterval(() => { if (Date.now() >= until) { stop(); return; } void load(); }, 5000);
    } catch (e) { if (mounted.current) setNotice(e instanceof Error ? e.message : 'Could not start solve.'); }
    finally { if (mounted.current) setSolving(false); }
  };
  const open = async (id: string) => {
    try { setOpened((cur) => ({ ...cur, [id]: null })); const row = await memory.get(id); setOpened((cur) => ({ ...cur, [id]: row })); setError(null); }
    catch (e) { setError(e); }
  };
  const dismiss = async (id: string) => {
    if (!(await confirmDialog('Dismiss this review item?'))) return;
    setBusy(id);
    try { await memory.dismiss(id); setItems((cur) => cur?.filter((i) => i.id !== id) ?? null); setCount(Math.max(0, count - 1)); onCount(Math.max(0, count - 1)); setError(null); }
    catch (e) { setError(e); }
    finally { setBusy(''); }
  };
  return (
    <div className="space-y-3" data-testid="memory-subview-review">
      {error != null && <div role="alert" className="text-xs text-red-600 dark:text-red-400">{describeError(error, 'read memory')}{!(error instanceof MemoryError && error.forbidden) && <button className={ghostBtn} onClick={load}>Try again</button>}</div>}
      {items === null && !error && <p className="text-xs text-slate-500">Loading…</p>}
      {items?.length === 0 && <p className="text-xs text-slate-500">Nothing waits for review.</p>}
      {items != null && items.length > 0 && <div className="space-y-1"><button className={primaryBtn} disabled={solving} onClick={solve}>Solve</button><p className="text-xs text-slate-500">Ling works through the queue and asks you only when it must.</p></div>}
      {notice && <div role="status" className="flex items-center gap-2 text-xs text-slate-500"><span>{notice}</span>{run && <OpenRun sessionId={run} />}</div>}
      {items?.map((item) => (
        <section key={item.id} className={`${sectionCls} space-y-2`}>
          <p data-no-i18n className="text-xs text-slate-800 dark:text-slate-200 whitespace-pre-wrap break-words">{item.note}</p>
          <div className="flex flex-wrap items-center gap-2"><span data-no-i18n className={chipCls}>{item.kind}</span><span className="text-xs text-slate-400">{fmtTime(item.created_at)}</span><button className={dangerBtn} disabled={!!busy} onClick={() => dismiss(item.id)}>Dismiss</button></div>
          <div className="flex flex-wrap gap-1">{item.row_ids?.map((id) => <button data-no-i18n key={id} className={ghostBtn} onClick={() => open(id)}>{id}</button>)}</div>
          {item.row_ids?.map((id) => opened[id] && <p data-no-i18n key={id} className="text-xs text-slate-600 dark:text-slate-300 whitespace-pre-wrap break-words">{opened[id]?.content}</p>)}
        </section>
      ))}
    </div>
  );
};
