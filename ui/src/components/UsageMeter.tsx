/**
 * Usage meter chip — the fuel gauge for the account's LLM allowance, shown
 * in the top bar of both shells (LauncherApp header, dev-console HeaderBar).
 *
 * Reads GET /api/account?app=linggen (the engine caches entitlement 60s):
 * one chip "6.8M left" for whichever allowance the account runs on — the
 * free trial (unsubscribed) or the monthly pool (subscribed). Amber past
 * 80% used or under 6 trial days remaining, red once exhausted.
 * Clicking opens a small dropdown with the progress bar, days left and a
 * link to linggen.dev billing.
 */
import React, { useState, useEffect, useRef, useCallback } from 'react';

interface TrialState {
  tokens: number;
  budget: number;
  started_at: number | null;
  expires_at: number | null;
  active: boolean;
}

interface UsageState {
  used: number;
  allowance: number;
  warn: boolean;
  over: boolean;
}

interface Meter {
  kind: 'trial' | 'monthly';
  used: number;
  total: number;
  daysLeft: number | null; // trial only; null = not started yet
  exhausted: boolean;
}

function fmtTokens(n: number): string {
  if (n >= 1e6) return `${(n / 1e6).toFixed(1).replace(/\.0$/, '')}M`;
  if (n >= 1e3) return `${Math.round(n / 1e3)}k`;
  return `${Math.max(0, Math.round(n))}`;
}

async function fetchMeter(): Promise<Meter | null> {
  const acc = await fetch('/api/account?app=linggen')
    .then((r) => (r.ok ? r.json() : null))
    .catch(() => null);
  if (!acc?.signed_in || !acc.entitlement) return null;

  if (acc.gate?.entitled) {
    const u: UsageState | undefined = acc.entitlement.usage;
    if (!u || !u.allowance) return null;
    return { kind: 'monthly', used: u.used, total: u.allowance, daysLeft: null, exhausted: u.over };
  }

  const t: TrialState | undefined = acc.gate?.trial;
  if (!t || !t.budget) return null;
  const daysLeft = t.expires_at
    ? Math.max(0, Math.ceil((t.expires_at - Date.now() / 1000) / 86400))
    : null;
  return { kind: 'trial', used: t.tokens, total: t.budget, daysLeft, exhausted: !t.active };
}

export const UsageMeter: React.FC = () => {
  const [meter, setMeter] = useState<Meter | null>(null);
  const [open, setOpen] = useState(false);
  const ref = useRef<HTMLDivElement>(null);

  const refresh = useCallback(() => {
    fetchMeter().then(setMeter);
  }, []);

  useEffect(() => {
    refresh();
    window.addEventListener('focus', refresh);
    const timer = setInterval(refresh, 120_000);
    return () => {
      window.removeEventListener('focus', refresh);
      clearInterval(timer);
    };
  }, [refresh]);

  useEffect(() => {
    if (!open) return;
    const handler = (e: MouseEvent) => {
      if (ref.current && !ref.current.contains(e.target as Node)) setOpen(false);
    };
    document.addEventListener('mousedown', handler);
    return () => document.removeEventListener('mousedown', handler);
  }, [open]);

  if (!meter) return null;

  const frac = meter.total > 0 ? Math.min(1, meter.used / meter.total) : 0;
  const low = frac >= 0.8 || (meter.daysLeft !== null && meter.daysLeft <= 5);
  const tone = meter.exhausted
    ? 'bg-red-500/10 text-red-600 hover:bg-red-500/20'
    : low
      ? 'bg-amber-500/10 text-amber-600 hover:bg-amber-500/20'
      : 'bg-slate-200 dark:bg-white/10 text-slate-500 dark:text-slate-400 hover:bg-slate-300 dark:hover:bg-white/20';
  const barTone = meter.exhausted ? 'bg-red-500' : low ? 'bg-amber-500' : 'bg-blue-500';
  const label = meter.exhausted
    ? meter.kind === 'trial' ? 'Trial ended' : 'Cap reached'
    : `${fmtTokens(meter.total - meter.used)} left`;
  const title = meter.kind === 'trial' ? 'Free trial' : 'Monthly usage';

  return (
    <div className="relative" ref={ref}>
      <button
        onClick={() => setOpen(!open)}
        title={title}
        className={`text-[10px] px-1.5 py-0.5 rounded font-medium whitespace-nowrap transition-colors ${tone}`}
      >
        {label}
      </button>
      {open && (
        <div className="absolute right-0 top-full mt-2 w-56 bg-white dark:bg-[#1a1a1a] border border-slate-200 dark:border-white/10 rounded-lg shadow-lg p-3 z-50">
          <div className="text-xs font-medium text-slate-700 dark:text-slate-300">{title}</div>
          <div className="mt-2 h-1.5 rounded-full bg-slate-100 dark:bg-white/10 overflow-hidden">
            <div className={`h-full rounded-full ${barTone}`} style={{ width: `${frac * 100}%` }} />
          </div>
          <div className="mt-1.5 text-[10px] text-slate-400">
            {fmtTokens(meter.used)} of {fmtTokens(meter.total)} tokens used
            {meter.kind === 'trial' && (
              meter.daysLeft === null
                ? ' · window starts on first use'
                : meter.exhausted
                  ? ''
                  : ` · ${meter.daysLeft} day${meter.daysLeft === 1 ? '' : 's'} left`
            )}
          </div>
          {meter.exhausted && (
            <div className="mt-1 text-[10px] text-red-500">
              {meter.kind === 'trial'
                ? 'Your free trial is used up — subscribe to keep going.'
                : 'Monthly allowance spent — add a top-up or wait for next month.'}
            </div>
          )}
          <a
            href="https://linggen.dev/app/billing"
            target="_blank"
            rel="noopener noreferrer"
            className="mt-2 block text-center text-xs font-medium text-blue-600 hover:text-blue-500 bg-blue-500/10 hover:bg-blue-500/15 rounded-md py-1.5 transition-colors"
          >
            {meter.exhausted ? 'Subscribe' : 'Manage plan'}
          </a>
        </div>
      )}
    </div>
  );
};
