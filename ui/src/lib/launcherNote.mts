/**
 * The line the launcher shows where its apps go: while the list loads, when
 * it failed (with a retry beside it), or when it came back empty — never
 * "Loading" for a list that is not coming. Pure, so node tests can hold it.
 */
export type ListState = 'loading' | 'ready' | 'error';

export function launcherNote(state: ListState, apps: number): string | null {
  if (state === 'loading') return 'Loading apps…';
  if (state === 'error') return "Apps didn't load.";
  return apps === 0 ? 'No apps installed.' : null;
}
