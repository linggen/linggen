/**
 * A draft belongs to its session. When the chat moves to another session the
 * box keeps what it held as that session's draft and shows the next one's,
 * so what was typed in one session is never sent into another; it is back
 * when the person returns. Kept for the page's life only. Pure, so node
 * tests can hold it.
 *
 * The box itself is never re-made for this: an input replaced while someone
 * types loses what they typed into the old one (found on a loaded machine,
 * 2026-11-19 — New chat's id arrived mid-typing).
 *
 * A chat still being made (New chat, before the engine gave it an id; or a
 * page not yet on any session) has the slot ''. Its words are that chat's:
 * when it becomes a session, the box keeps them.
 */

export type Draft = {
  text: string;
  images: { id: number; data: string }[];
  /** Whom the message goes to (the chip); empty, the chat's default. */
  addressee: string;
};

const EMPTY: Draft = { text: '', images: [], addressee: '' };
const drafts = new Map<string, Draft>();

const keyOf = (session: string | null | undefined) => session || '';

/** A box with no words and no pictures holds no draft: a chip alone may be
 *  the pre-fill's, which the session sets again itself. */
const isEmpty = (draft: Draft) => !draft.text && draft.images.length === 0;

/** The slot the chat box is for: the open session, or '' while New chat is
 *  being made (the new chat's, whatever session was open before). */
export function boxSlot(session: string | null | undefined, makingNewChat: boolean): string {
  return makingNewChat ? '' : keyOf(session);
}

/**
 * The box moves from slot `from` to slot `to` holding `holding`; returns
 * what it holds next.
 *  - the chat being made became a session (`from` ''): it keeps its words;
 *  - otherwise `from` keeps them as its draft, and the box shows `to`'s —
 *    none for a New chat, which is a fresh one.
 */
export function switchDraft(from: string, to: string, holding: Draft): Draft {
  if (from === to) return holding;
  if (from === '') {
    if (!isEmpty(holding)) return holding;
    return drafts.get(to) ?? EMPTY;
  }
  if (isEmpty(holding)) drafts.delete(from);
  else drafts.set(from, holding);
  // Read, not taken: a render run twice (React's strict mode) finds it
  // again; leaving `to` writes over it.
  return to === '' ? EMPTY : drafts.get(to) ?? EMPTY;
}
