/** The refill time in `BUDGET_EMPTY: refill_at=…` (optionally `Error:`-prefixed),
 *  0 when it names none, null for any other text. */
export function parseBudgetEmpty(text?: string): number | null {
  if (!text) return null;
  const body = text.trim().replace(/^Error:\s*/i, '');
  if (!body.startsWith('BUDGET_EMPTY:')) return null;
  const m = /refill_at=(\d+)/.exec(body);
  return m ? Number(m[1]) : 0;
}
