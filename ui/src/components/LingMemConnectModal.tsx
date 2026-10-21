import React, { useCallback, useEffect, useState } from 'react';

// A machine on the LAN asking ling-mem to share memory. The engine forwards
// the three calls (`/api/ling-mem/connect/*`) only for someone at this Mac —
// anyone else gets 403 and this renders nothing, so the server decides who
// sees it, never the page.

const POLL_MS = 3000;

interface Pending {
  id: string;
  device_name: string;
  ip: string;
  known: boolean;
}

const overlayCls = 'fixed inset-0 z-[1000] flex items-center justify-center bg-black/40';
const cardCls =
  'w-80 max-w-[90vw] rounded-xl bg-white dark:bg-[#1c1c1c] border border-slate-200 dark:border-white/10 shadow-xl p-4';
const msgCls = 'text-sm text-slate-800 dark:text-slate-200 break-words';
const hintCls = 'mt-1 text-xs text-slate-500 dark:text-slate-400';
const denyCls =
  'px-3 py-1.5 rounded-lg text-xs font-bold text-slate-600 dark:text-slate-300 hover:bg-slate-100 dark:hover:bg-white/5';
const allowCls = 'px-3 py-1.5 rounded-lg text-xs font-bold bg-blue-600 text-white hover:bg-blue-700';

async function fetchPending(): Promise<Pending | null | 'refused'> {
  const resp = await fetch('/api/ling-mem/connect/pending', { cache: 'no-store' });
  if (resp.status === 403) return 'refused';
  if (!resp.ok) return null;
  const json = await resp.json();
  return json?.data?.pending ?? null;
}

export const LingMemConnectModal: React.FC = () => {
  const [pending, setPending] = useState<Pending | null>(null);

  useEffect(() => {
    let stopped = false;
    let timer: ReturnType<typeof setTimeout> | undefined;
    const tick = async () => {
      let next: Pending | null | 'refused' = null;
      try {
        next = await fetchPending();
      } catch { /* engine or ling-mem down — try again */ }
      if (stopped || next === 'refused') return;
      setPending(next);
      timer = setTimeout(tick, POLL_MS);
    };
    tick();
    return () => {
      stopped = true;
      if (timer) clearTimeout(timer);
    };
  }, []);

  const decide = useCallback(async (verdict: 'allow' | 'deny') => {
    if (!pending) return;
    setPending(null);
    try {
      await fetch(`/api/ling-mem/connect/${encodeURIComponent(pending.id)}/${verdict}`, { method: 'POST' });
    } catch { /* the next poll shows the truth */ }
  }, [pending]);

  if (!pending) return null;
  return (
    <div className={overlayCls}>
      <div className={cardCls} role="alertdialog" aria-modal="true" aria-label="Share memory">
        <p className={msgCls}>
          <b>{pending.device_name}</b> ({pending.ip}) wants to share memory.
        </p>
        {pending.known && <p className={hintCls}>Allowed before; it lost its key.</p>}
        <div className="flex justify-end gap-2 mt-4">
          <button className={denyCls} onClick={() => decide('deny')}>Deny</button>
          <button className={allowCls} autoFocus onClick={() => decide('allow')}>Allow</button>
        </div>
      </div>
    </div>
  );
};
