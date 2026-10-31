/**
 * A page left open across an engine update keeps running the old bundle —
 * fixes shipped since never reach it. page_state carries the UI build the
 * engine serves now; the first one a page sees is the build it loaded, and
 * once they differ the page reloads as soon as it is idle: no reply being
 * written in the session it shows and nothing typed in any field.
 */

/** How often a page that is out of date looks for an idle moment. */
export const IDLE_CHECK_MS = 3_000;

export type BuildState = { loaded: string | null; stale: boolean };

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

let state: BuildState = { loaded: null, stale: false };
let timer: ReturnType<typeof setInterval> | null = null;

/** Note the build a page_state names; an out-of-date page waits to reload. */
export function noteUiBuild(served: string | undefined, isIdle: () => boolean): void {
  state = noteServed(state, served);
  if (!state.stale || timer) return;
  timer = setInterval(() => {
    const fields = document.querySelectorAll<HTMLInputElement | HTMLTextAreaElement | HTMLElement>(
      'textarea, input[type="text"], input:not([type]), [contenteditable="true"]',
    );
    if (hasTypedText(fields as Iterable<HTMLInputElement>) || !isIdle()) return;
    if (timer) clearInterval(timer);
    location.reload();
  }, IDLE_CHECK_MS);
}
