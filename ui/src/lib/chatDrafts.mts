/**
 * A draft belongs to its session. The chat box is made anew for each session
 * (ChatPanel keys it by `boxSlot`), so what was typed in one session is
 * never sent into another; what the box held when the person left a session
 * waits here and is back when they return to it. Kept for the page's life
 * only. Pure, so node tests can hold it.
 *
 * A chat still being made (New chat, before the engine gave it an id; or a
 * page not yet on any session) has the slot ''. Its words are that chat's:
 * the box made for the session it becomes takes them.
 */

export type Draft = {
  text: string;
  images: { id: number; data: string }[];
  /** Whom the message goes to (the chip); empty, the chat's default. */
  addressee: string;
};

const drafts = new Map<string, Draft>();
/** The slot of the box open now. */
let open: string | null = null;

const keyOf = (session: string | null | undefined) => session || '';

/** The slot a chat box is for: the open session, or '' while New chat is
 *  being made (the new chat's, whatever session was open before). */
export function boxSlot(session: string | null | undefined, makingNewChat: boolean): string {
  return makingNewChat ? '' : keyOf(session);
}

/** Keep what the box holds as its slot's draft — on every change, so the
 *  next box opened finds it. A box with no words and no pictures keeps none:
 *  a chip alone may be the pre-fill's, which the session sets again itself. */
export function keepDraft(slot: string | null | undefined, draft: Draft): void {
  if (!draft.text && draft.images.length === 0) {
    drafts.delete(keyOf(slot));
    return;
  }
  drafts.set(keyOf(slot), draft);
}

/** The draft a box for `slot` opens on: the slot's own; a box for a
 *  session opened straight after the chat still being made takes that
 *  chat's words (they become the session's). A box for '' opens empty: a
 *  New chat is a fresh one. */
export function openDraft(slot: string | null | undefined): Draft | undefined {
  const key = keyOf(slot);
  const before = open;
  open = key;
  if (!key) {
    if (before !== '') drafts.delete('');
    return drafts.get('');
  }
  const own = drafts.get(key);
  if (own || before !== '') return own;
  const made = drafts.get('');
  if (!made) return undefined;
  // Moved, not copied: opening twice (React's strict re-render) finds it.
  drafts.delete('');
  drafts.set(key, made);
  return made;
}
