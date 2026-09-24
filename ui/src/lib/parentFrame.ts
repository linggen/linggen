/**
 * The page that frames this one — a skill page, the launcher — is one of the
 * engine's own, served from this same origin (through the linggen.dev tunnel,
 * a blob of the site's origin: still the same as ours). Messages go only to
 * that origin, never '*', and only its messages are acted on: another site
 * framing the chat could otherwise make it send.
 */

/** Post to the parent frame, if framed; a cross-origin parent never hears it. */
export function postToParent(msg: unknown): void {
  if (window.parent === window) return;
  try {
    window.parent.postMessage(msg, window.location.origin);
  } catch {
    /* not framed in a way we can reach */
  }
}

/** Whether `e` came from our own parent frame, on our own origin. */
export function fromParent(e: MessageEvent): boolean {
  return e.source === window.parent && e.origin === window.location.origin;
}
