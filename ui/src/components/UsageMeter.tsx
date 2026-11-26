/**
 * Linggen Cloud's allowance as one line of words, never a token count or a
 * price: "Used this month: 23%" (a trial reads "Trial used: 23%"), amber past
 * 80%, red once spent. The developer account is metered nowhere and says so.
 * Read from GET /api/account?app=linggen, which hands on the site's
 * entitlement as it is (`usage.percent`, `usage.resets_at`; the engine caches
 * it 60s). The collapsed Linggen Cloud row draws `text`; the opened row adds
 * the link to billing (`UsageLink`).
 */
import React, { useState, useEffect, useCallback } from 'react';
import { W } from '../lib/modelsWords.mts';
import { linkBtn } from './models/shell';
import { account } from '../lib/endpoints';
import { cloudUsage, type CloudUsage } from '../lib/cloudUsage.mts';

/** The account's use, read now and again on focus and every two minutes. */
export function useCloudUsage(signedIn: boolean): CloudUsage | null {
  const [usage, setUsage] = useState<CloudUsage | null>(null);
  const refresh = useCallback(() => {
    account.get('linggen').catch(() => null).then((acc) => setUsage(cloudUsage(acc)));
  }, []);
  useEffect(() => {
    if (!signedIn) { setUsage(null); return; }
    refresh();
    window.addEventListener('focus', refresh);
    const timer = setInterval(refresh, 120_000);
    return () => {
      window.removeEventListener('focus', refresh);
      clearInterval(timer);
    };
  }, [signedIn, refresh]);
  return usage;
}

export const usageToneCls = (tone: CloudUsage['tone']) =>
  tone === 'spent' ? 'text-red-600' : tone === 'warn' ? 'text-amber-600' : '';

/** The link to the plan: Subscribe once spent, Manage plan otherwise. */
export const UsageLink: React.FC<{ usage: CloudUsage }> = ({ usage }) => (
  <a data-testid="cloud-usage-link" href="https://linggen.dev/app/billing" target="_blank" rel="noopener noreferrer" className={linkBtn}>
    {usage.tone === 'spent' ? W.subscribe : W.managePlan}
  </a>
);
