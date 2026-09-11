import React from 'react';

/** Shown when a turn is refused because a skill's play window is spent —
 *  the engine's `BUDGET_EMPTY: refill_at=<unix secs>` for skills that declare
 *  a `cloud.meter`. A pace, not a paywall: nothing to buy, only when it frees up.
 */
export const BudgetEmptyBlock: React.FC<{ refillAt: number }> = ({ refillAt }) => {
  const when = refillAt > 0
    ? new Date(refillAt * 1000).toLocaleTimeString([], { hour: 'numeric', minute: '2-digit' })
    : null;
  return (
    <div className="rounded-lg border border-amber-300 dark:border-amber-800 bg-amber-50 dark:bg-amber-950/40 px-4 py-3 text-sm text-amber-900 dark:text-amber-200">
      Play time is used up for now{when ? <> — it frees up at <b>{when}</b></> : null}.
    </div>
  );
};

/** The refill time in `BUDGET_EMPTY: refill_at=…` (optionally `Error:`-prefixed),
 *  0 when it names none, null for any other text. */
export function parseBudgetEmpty(text?: string): number | null {
  if (!text) return null;
  const body = text.trim().replace(/^Error:\s*/i, '');
  if (!body.startsWith('BUDGET_EMPTY:')) return null;
  const m = /refill_at=(\d+)/.exec(body);
  return m ? Number(m[1]) : 0;
}
