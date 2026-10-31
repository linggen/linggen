/**
 * A page left open across an engine update keeps running the old bundle —
 * fixes shipped since never reach it. page_state carries the UI build the
 * engine serves now; the first one a page sees is the build it loaded, and
 * once they differ the page reloads as soon as it is idle: no reply being
 * written in the session it shows, no question or plan waiting for an
 * answer, no dialog open and nothing typed in any field.
 */

/** How often a page that is out of date looks for an idle moment. */
export const IDLE_CHECK_MS = 3_000;

export type BuildState = { loaded: string | null; stale: boolean };

/** What the stores know about the page (the DOM adds typed text and dialogs
 *  no store holds). */
export type PageFacts = {
  /** A reply is being written in the session shown. */
  replying: boolean;
  /** An ask or plan card waits for the person's answer. */
  awaitingAnswer: boolean;
  /** A modal, picker or panel is open. */
  dialogOpen: boolean;
};

/** Every field a person types in. */
export const TEXT_FIELDS = [
  'textarea',
  'input:not([type])',
  ...['text', 'search', 'email', 'number', 'url', 'tel', 'password'].map((t) => `input[type="${t}"]`),
  '[contenteditable]:not([contenteditable="false"])',
].join(', ');

/** Dialogs a component keeps in its own state. */
export const OPEN_DIALOGS = 'dialog[open], [aria-modal="true"], [role="dialog"], [role="alertdialog"]';

/** The page's build state after a page_state naming `served`. */
export function noteServed(state: BuildState, served: string | undefined): BuildState {
  if (!served) return state;
  if (state.loaded === null) return { loaded: served, stale: false };
  return { loaded: state.loaded, stale: state.stale || served !== state.loaded };
}

/** True when any text field on the page holds something typed. */
export function hasTypedText(fields: Iterable<{ value?: string; textContent?: string | null; isContentEditable?: boolean }>): boolean {
  for (const field of fields) {
    const text = field.isContentEditable ? field.textContent : field.value;
    if (text && text.trim()) return true;
  }
  return false;
}

/** A reload now would cut nothing short and lose nothing. */
export function idleForReload(facts: PageFacts, typed: boolean): boolean {
  return !facts.replying && !facts.awaitingAnswer && !facts.dialogOpen && !typed;
}

let state: BuildState = { loaded: null, stale: false };
let timer: ReturnType<typeof setInterval> | null = null;

/** Note the build a page_state names; an out-of-date page waits to reload. */
export function noteUiBuild(served: string | undefined, pageFacts: () => PageFacts): void {
  state = noteServed(state, served);
  if (!state.stale || timer) return;
  timer = setInterval(() => {
    const facts = pageFacts();
    const fields = document.querySelectorAll<HTMLElement>(TEXT_FIELDS);
    const dialogOpen = facts.dialogOpen || document.querySelector(OPEN_DIALOGS) !== null;
    if (!idleForReload({ ...facts, dialogOpen }, hasTypedText(fields as Iterable<HTMLInputElement>))) return;
    if (timer) clearInterval(timer);
    location.reload();
  }, IDLE_CHECK_MS);
}
