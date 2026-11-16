import React, { useCallback, useEffect, useRef, useState } from 'react';
import { startDream } from './dreamApi.mts';
import { describeError, memory, MemoryError, type MemoryDays as Days } from './memoryApi.mts';
import { ghostBtn, primaryBtn, sectionCls, tierChipCls } from './memoryStyle';

export const MemoryDays: React.FC = () => {
  const [data, setData] = useState<Days | null>(null);
  const [error, setError] = useState<unknown>(null);
  const [notice, setNotice] = useState('');
  const [busy, setBusy] = useState(false);
  const timer = useRef<ReturnType<typeof setInterval> | null>(null);
  const stop = useCallback(() => { if (timer.current) clearInterval(timer.current); timer.current = null; }, []);
  const load = useCallback(async () => {
    try { setData(await memory.days()); setError(null); } catch (e) { setError(e); }
  }, []);
  useEffect(() => { void load(); return stop; }, [load, stop]);
  const start = async (day?: string) => {
    setBusy(true); setNotice('');
    try {
      await startDream(day); setNotice('Dream started.'); void load(); stop();
      const until = Date.now() + 180_000;
      timer.current = setInterval(() => { if (Date.now() >= until) { stop(); return; } void load(); }, 5000);
    } catch (e) { setNotice(e instanceof Error ? e.message : 'Could not start dream.'); }
    finally { setBusy(false); }
  };
  const days = [...(data?.days ?? [])].sort((a, b) => b.date.localeCompare(a.date));
  const waiting = days.filter((d) => d.state !== 'dreamed').length;
  return (
    <div className="space-y-3" data-testid="memory-subview-days">
      <button className={primaryBtn} disabled={busy} onClick={() => start()}>Dream now</button>
      {notice && <p role="status" className="text-xs text-slate-500">{notice}</p>}
      {error != null && <div role="alert" className="text-xs text-red-600 dark:text-red-400">{describeError(error, 'read memory')}{!(error instanceof MemoryError && error.forbidden) && <button className={ghostBtn} onClick={load}>Try again</button>}</div>}
      {!data && !error && <p className="text-xs text-slate-500">Loading…</p>}
      {data && <p className="text-xs text-slate-500">{`${waiting} days wait for a dream.`}</p>}
      {data && days.length === 0 && <p className="text-xs text-slate-500">No days yet.</p>}
      {days.map((day) => <section key={day.date} className={`${sectionCls} flex flex-wrap items-center gap-2`} data-testid="memory-day">
        <span className="text-xs font-bold text-slate-700 dark:text-slate-300">{day.date}</span>
        <span className={tierChipCls(day.state === 'dreamed' ? 'episodic' : 'core')}>{day.state}</span>
        <span className="text-xs text-slate-500">{`${day.rows} rows · ${day.unjudged} unjudged`}</span>
        {day.promoted != null && <span className="text-xs text-slate-500">{`${day.promoted} promoted`}</span>}
        {day.forgotten != null && <span className="text-xs text-slate-500">{`${day.forgotten} forgotten`}</span>}
        {day.state !== 'dreamed' && <button className={ghostBtn} disabled={busy} onClick={() => start(day.date)}>Dream this day</button>}
      </section>)}
    </div>
  );
};
