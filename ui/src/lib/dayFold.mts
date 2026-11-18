/**
 * The day's session on one page (`doc/one-ling-spec.md` § Linggen's daily session): every app's chat is one thread, and the stretches written in other
 * places than this page's are folded to one line each — the app and how
 * many messages — which opens on a tap. A private app's talk never reaches
 * a page outside it (the engine leaves it out), so it is never folded here.
 *
 * A row names its place only in a day's session (`place`: an app's name, or
 * `:home` for the main page and Yinyue's own surfaces). A live row, not
 * reloaded yet, takes the place the engine told on its events: a person's
 * message on its own event, a run's rows from their run (`withLivePlaces`),
 * so another place's turn folds while it streams. Rows without one — any
 * other session — are never folded.
 */

export const HOME_PLACE = ':home';

/** One item of the list as drawn: a message, or a folded stretch. */
export type FoldItem =
  | { kind: 'message'; index: number }
  | { kind: 'fold'; place: string; indexes: number[] };

/** The page's own place: its app, or home. */
export function pagePlace(skill: string | null | undefined): string {
  return skill || HOME_PLACE;
}

/** A skill as the page lists it: its name, and its app's title if it has one. */
type NamedSkill = { name: string; app?: { title?: string | null } | null };

/** What a place is called: its app's display name (its title, else its
 *  name), or Linggen for home. */
export function placeName(place: string, skills: ReadonlyArray<NamedSkill> = []): string {
  if (place === HOME_PLACE) return 'Linggen';
  return skills.find((s) => s.name === place)?.app?.title || place;
}

/** The place the engine told for each run (on its events, its page-state
 *  row), the first word kept: a late event of an ended run may arrive while
 *  the next one runs. The same object when nothing new was told. */
export function toldPlaces(
  known: Record<string, string>,
  told: ReadonlyArray<{ run_id?: string | null; place?: string | null }>,
): Record<string, string> {
  let out = known;
  for (const { run_id, place } of told) {
    if (!run_id || !place || out[run_id]) continue;
    if (out === known) out = { ...known };
    out[run_id] = place;
  }
  return out;
}

/** The place the running turn stands in when it is not this page's: only
 *  as the engine told it; a run with no place told is this page's, as before. */
export function runningElsewhere(runId: string | null | undefined, places: Record<string, string>, here: string): string | null {
  const place = runId ? places[runId] : undefined;
  return place && place !== here ? place : null;
}

/** Whether an app's own scripts are handed what a run sends (its stream,
 *  its blocks, its end, its signs of life) at a page standing in `here`:
 *  only a run standing in this page's place. The place is the run's as the
 *  engine first told it, else the one the event names; with none told (the
 *  setting off, any session not a day) every run is handed on, as before. */
export function streamsHere(
  told: { run_id?: string | null; place?: string | null },
  places: Record<string, string>,
  here: string,
): boolean {
  const place = (told.run_id ? places[told.run_id] : undefined) || told.place;
  return !place || place === here;
}

/** The status line while another place's turn runs. */
export function workingIn(name: string): string {
  return `Working in ${name}`;
}

/** The Stop control while another place's turn runs: it still stops it. */
export function stopLabel(name: string): string {
  return `Stop · ${name}`;
}

/** `messages` (with their places) as drawn on a page standing in `here`:
 *  each run of messages written in another place is one fold. */
export function foldOthers(messages: ReadonlyArray<{ place?: string }>, here: string): FoldItem[] {
  const out: FoldItem[] = [];
  messages.forEach((m, index) => {
    const elsewhere = m.place && m.place !== here ? m.place : null;
    if (!elsewhere) {
      out.push({ kind: 'message', index });
      return;
    }
    const last = out[out.length - 1];
    if (last?.kind === 'fold' && last.place === elsewhere) last.indexes.push(index);
    else out.push({ kind: 'fold', place: elsewhere, indexes: [index] });
  });
  return out;
}

/** Rows with the place the engine told for the run that wrote them, when
 *  they name none yet (a live row, not reloaded): another place's turn then
 *  folds while it streams, as its saved stretch does. The same list when
 *  nothing changes. */
export function withLivePlaces<T extends { place?: string; runId?: string }>(
  messages: T[],
  places: Record<string, string>,
): T[] {
  const told = (m: T) => (!m.place && m.runId ? places[m.runId] : undefined);
  if (!messages.some(told)) return messages;
  return messages.map((m) => {
    const place = told(m);
    return place ? { ...m, place } : m;
  });
}

/** A folded stretch's line: its place, and how many messages — or that it
 *  is working, while its turn streams. */
export function foldLabel(name: string, count: number, working: boolean): string {
  if (working) return `${name} · working…`;
  return count === 1 ? `${name} · 1 message` : `${name} · ${count} messages`;
}

/** Whether a waiting message is this page's to change: sent from here, or
 *  with no place told (any other session). Only its own page offers Edit,
 *  Remove and Steer; another page shows where it waits from. */
export function queuedHere(item: { place?: string | null }, here: string): boolean {
  return !item.place || item.place === here;
}

/** A waiting message sent from another place, as another page shows it. */
export function waitingFrom(name: string): string {
  return `Waiting from ${name}`;
}

/** Whether Clear on a page cancels the run going on: never in the day's
 *  session, where that run may be another place's and Clear takes only this
 *  page's part (Stop is the button that stops it). */
export function clearCancelsRun(session: { day?: boolean } | undefined): boolean {
  return !session?.day;
}
