/**
 * What a page_state push may change. Its scoped part (the queue, the
 * background tasks, the permission view, the runs list) is about one session,
 * `ps.session_id`: it is written only while that session is the one open,
 * and its runs list says nothing about any other session's sends. Without
 * this, a push for the session just left overwrote the open one's queue,
 * and a push for one session retired every other session's pending send.
 * Pure, so node tests can hold it.
 */
import { stillOn } from './sessionLoad.mts';

/** The push's scoped part may be written: it is for the open session (a
 *  push that names no session is the page's own, applied as it is). */
export function scopedHere(pushFor: string | null | undefined, open: string | null | undefined): boolean {
  return !pushFor || (!!open && stillOn(pushFor, open));
}

/**
 * The sessions whose pending send a runs list retires: only the session the
 * list is for (`runsFor` — the engine lists that one session's runs), when
 * no top-level run of it is running and its send is older than `staleMs`.
 */
export function retiredSends(
  pending: Record<string, number>,
  running: Set<string | undefined>,
  runsFor: string | undefined,
  now: number,
  staleMs: number,
): string[] {
  return Object.entries(pending)
    .filter(([sid, sentAt]) => sid === runsFor && !running.has(sid) && now - sentAt > staleMs)
    .map(([sid]) => sid);
}
