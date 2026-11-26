/** Linggen Cloud's use as one line of words (the pure half of components/UsageMeter). */
import { W, say } from './modelsWords.mts';
import type { Account } from './endpoints';

export interface CloudUsage {
  text: string;
  tone: 'ok' | 'warn' | 'spent';
  /** The developer account has no plan to manage. */
  metered: boolean;
}

const percentOf = (used: number, total: number) => (total > 0 ? Math.min(100, Math.round((used / total) * 100)) : 0);

/** What the account says about use: null when signed out or nothing is known yet. */
export function cloudUsage(acc: Account | null | undefined): CloudUsage | null {
  if (!acc?.signed_in || !acc.entitlement) return null;
  if (acc.gate?.developer || acc.entitlement.developer) return { text: W.usageDeveloper, tone: 'ok', metered: false };
  if (acc.gate?.entitled) {
    const u = acc.entitlement.usage;
    if (!u) return null;
    const percent = u.percent ?? percentOf(u.used, u.allowance);
    return { text: say(W.usageMonth, { percent }), tone: u.over || percent >= 100 ? 'spent' : percent >= 80 ? 'warn' : 'ok', metered: true };
  }
  const t = acc.gate?.trial;
  if (!t) return null;
  const percent = t.percent ?? percentOf(t.tokens, t.budget);
  return { text: say(W.usageTrial, { percent }), tone: !t.active ? 'spent' : percent >= 80 ? 'warn' : 'ok', metered: true };
}
