/**
 * When a presence beat goes, and what it counts as told. A covered surface
 * (the desktop app's tab over the chat) beats only to withdraw a focused
 * reading the engine still holds; that withdrawal counts as told only when
 * the engine took it, so a request that fails is sent again on the next
 * interval rather than leaving the engine sure the person is here. Pure, so
 * node tests can hold it; `presence.ts` wires it to the page.
 */

export interface PresenceReading {
  focused: boolean;
}

export interface BeatDeps<R extends PresenceReading> {
  /** The data channel is up; a beat while it is down is skipped, not queued. */
  isOpen: () => boolean;
  /** The app tab covers this surface. */
  covered: () => boolean;
  /** This surface's reading now. */
  read: () => R;
  /** Send it; true only when the engine took it. */
  send: (reading: R) => Promise<boolean>;
}

/** A beat function: sends one reading when one is due. */
export function presenceBeat<R extends PresenceReading>(deps: BeatDeps<R>): () => Promise<void> {
  // The focus the engine last took from this surface.
  let toldFocused = false;
  return async () => {
    if (!deps.isOpen() || (deps.covered() && !toldFocused)) return;
    const reading = deps.read();
    const took = await deps.send(reading).catch(() => false);
    if (took) toldFocused = reading.focused;
  };
}
