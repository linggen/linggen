/**
 * A draft belongs to its session. The chat box is made anew for each session
 * (ChatPanel keys it by the session), so what was typed in one session is
 * never sent into another; what the box held when the person left a session
 * waits here and is back when they return to it. Kept for the page's life
 * only. Pure, so node tests can hold it.
 */

export type Draft = {
  text: string;
  images: { id: number; data: string }[];
  /** Whom the message goes to (the chip); empty, the chat's default. */
  addressee: string;
};

const drafts = new Map<string, Draft>();

const keyOf = (session: string | null | undefined) => session || '';

/** Keep what the box holds as `session`'s draft. A box with no words and no
 *  pictures keeps none: a chip alone may be the pre-fill's, which the
 *  session sets again itself. */
export function keepDraft(session: string | null | undefined, draft: Draft): void {
  if (!draft.text && draft.images.length === 0) {
    drafts.delete(keyOf(session));
    return;
  }
  drafts.set(keyOf(session), draft);
}

/** The draft left in `session`, if any. */
export function draftOf(session: string | null | undefined): Draft | undefined {
  return drafts.get(keyOf(session));
}
