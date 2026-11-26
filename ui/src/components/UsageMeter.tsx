/**
 * Usage line — the person's Linggen Cloud allowance as one line, shown under
 * the Linggen Cloud block in Settings → Models: "Used this month: 1.2M / 10M"
 * (a trial reads "Trial used: …"), amber past 80%, red once spent, with a link
 * to linggen.dev billing. Reads GET /api/account?app=linggen (the engine caches
 * entitlement 60s). The developer account is metered nowhere: no line.
 */
import React, { useState, useEffect, useCallback } from 'react';
import { W, say } from '../lib/modelsWords.mts';
import { linkBtn } from './models/shell';
import { account, type TrialState, type UsageState } from '../lib/endpoints';

interface Meter {
  kind: 'trial' | 'monthly';
  used: number;
  total: number;
  exhausted: boolean;
  /** A subscription existed and ended — the account is back on its trial remainder. */
  pastPlan: boolean;
}

function fmtTokens(n: number): string {
  if (n >= 1e6) return `${(n / 1e6).toFixed(1).replace(/\.0$/, '')}M`;
  if (n >= 1e3) return `${Math.round(n / 1e3)}k`;
  return `${Math.max(0, Math.round(n))}`;
}

async function fetchMeter(): Promise<Meter | null> {
  const acc = await account.get('linggen').catch(() => null);
  if (!acc?.signed_in || !acc.entitlement) return null;
  // The developer account is metered nowhere: no meter to draw.
  if (acc.gate?.developer) return null;

  if (acc.gate?.entitled) {
    const u: UsageState | undefined = acc.entitlement.usage;
    if (!u || !u.allowance) return null;
    return { kind: 'monthly', used: u.used, total: u.allowance, exhausted: u.over, pastPlan: false };
  }

  const t: TrialState | null | undefined = acc.gate?.trial;
  if (!t || !t.budget) return null;
  const pastPlan = Object.keys(acc.entitlement.apps ?? {}).length > 0;
  return { kind: 'trial', used: t.tokens, total: t.budget, exhausted: !t.active, pastPlan };
}

export const UsageMeter: React.FC = () => {
  const [meter, setMeter] = useState<Meter | null>(null);
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

  if (!meter) return null;

  const frac = meter.total > 0 ? Math.min(1, meter.used / meter.total) : 0;
  const tone = meter.exhausted ? 'text-red-600' : frac >= 0.8 ? 'text-amber-600' : 'text-slate-500 dark:text-slate-400';
  const words = meter.kind === 'trial' ? W.usageTrial : W.usageMonth;
  return (
    <p data-testid="cloud-usage" className={`flex flex-wrap items-center gap-x-3 text-[11px] ${tone}`}>
      <span>{say(words, { used: fmtTokens(meter.used), cap: fmtTokens(meter.total) })}</span>
      <a href="https://linggen.dev/app/billing" target="_blank" rel="noopener noreferrer" className={linkBtn}>
        {meter.exhausted ? W.subscribe : W.managePlan}
      </a>
    </p>
  );
};
