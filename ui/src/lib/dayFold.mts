/**
 * The day's session on one page (`doc/one-ling-spec.md` § One session a
 * day): every app's chat is one thread, and the stretches written in other
 * places than this page's are folded to one line each — the app and how
 * many messages — which opens on a tap. A private app's talk never reaches
 * a page outside it (the engine leaves it out), so it is never folded here.
 *
 * A row names its place only in a day's session (`place`: an app's name, or
 * `:home` for the main page and Yinyue's own surfaces); rows without one —
 * any other session, or a live message not reloaded yet — are never folded.
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

/** What a folded stretch is called: its app, or Linggen for home. */
export function placeName(place: string): string {
  return place === HOME_PLACE ? 'Linggen' : place;
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
