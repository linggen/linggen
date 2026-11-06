import { useEffect, useState } from 'react';
import { apiGet } from '../lib/api';

type Window = { used_percent: number; limit_window_seconds: number; reset_at?: number };
type Usage = { rate_limit?: { primary_window?: Window | null; secondary_window?: Window | null } | null; credits?: { balance?: string | number | null } | null };

// Share requests across the chat and model rows.
let pending: Promise<Usage | null> | undefined;
let cached: Usage | null = null;
let fetchedAt = 0;
function readUsage(): Promise<Usage | null> {
  if (Date.now() - fetchedAt < 60_000) return Promise.resolve(cached);
  if (!pending) {
    pending = apiGet<Usage>('/api/auth/codex/usage').catch(() => null).then(value => {
      cached = value;
      fetchedAt = Date.now();
      pending = undefined;
      return value;
    });
  }
  return pending;
}

export function CodexUsage() {
  const [usage, setUsage] = useState<Usage | null>(cached);
  useEffect(() => {
    let alive = true;
    const refresh = () => { void readUsage().then(value => { if (alive) setUsage(value); }); };
    refresh();
    const timer = window.setInterval(refresh, 60_000);
    return () => { alive = false; window.clearInterval(timer); };
  }, []);
  const windows = [usage?.rate_limit?.primary_window, usage?.rate_limit?.secondary_window]
    .filter((w): w is Window => !!w && typeof w.used_percent === 'number');
  if (!windows.length) return null;
  return <span className="ml-2 text-slate-500" title="Codex account allowance remaining">
    {windows.map(w => {
      const label = w.limit_window_seconds === 18000 ? '5h' : w.limit_window_seconds === 604800 ? '7d' : `${Math.round(w.limit_window_seconds / 3600)}h`;
      const reset = w.reset_at ? `; resets ${new Date(w.reset_at * 1000).toLocaleString()}` : '';
      return <span key={w.limit_window_seconds} className="ml-1" title={`${label} allowance${reset}`}>{label} {Math.max(0, Math.min(100, 100 - w.used_percent))}%</span>;
    })}
    {usage?.credits?.balance != null && <span className="ml-1">Credits {usage.credits.balance}</span>}
  </span>;
}
