/**
 * A change shown at once and then saved is taken back when its save fails:
 * each field it set returns to what it was before — unless a newer change
 * has set that field since, which stands. Pure, so node tests can hold it.
 */
export function undoUpdate<T extends Record<string, unknown>>(
  current: T,
  before: T,
  update: Partial<T>,
): T {
  const next = { ...current };
  for (const key of Object.keys(update) as (keyof T)[]) {
    if (current[key] === update[key]) next[key] = before[key];
  }
  return next;
}
