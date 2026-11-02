/**
 * How a page dials the engine again when a dial does not come up.
 *
 * A dial that got its answer but whose control channel never opens used to
 * wait for the browser to call ICE 'failed' — 30 s and more — and the
 * reconnect after it backed off up to 30 s. A skill page's call gives up
 * after 20 s (CONNECT_WAIT_MS), so one stuck dial was a page stuck on
 * "not connected" for a minute (2026-11-02, Lingjing under load). Now each
 * dial has a deadline of its own, and a local redial comes quickly: two or
 * three fresh peers fit in one call's wait. Pure, so node tests hold it.
 */

/** How long one dial may take, offer to open control channel. Local: ICE
 *  gathering (capped at 5 s) plus one signaling round trip; remote: the
 *  relay's polling and TURN as well. */
export function dialDeadlineMs(remote: boolean): number {
  return remote ? 25_000 : 8_000;
}

/** The wait before redial `attempt` (0-based). Local dials are a loopback
 *  POST, cheap to repeat: 0.5, 1, 2, 4, then 5 s. Remote: 1 s doubling to 30 s. */
export function redialDelayMs(attempt: number, remote: boolean): number {
  const base = remote ? 1000 : 500;
  const cap = remote ? 30_000 : 5_000;
  return Math.min(base * 2 ** attempt, cap);
}

/** Whether a page coming back into view should dial now instead of waiting
 *  out its backoff: it is not connected and no dial is in flight. A hidden tab
 *  under load can sit frozen through a whole wait. */
export function dialOnVisible(status: 'disconnected' | 'connecting' | 'connected' | 'reconnecting', intentional: boolean): boolean {
  return !intentional && status === 'reconnecting';
}
