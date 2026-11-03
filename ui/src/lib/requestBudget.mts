/**
 * How long one call over the control channel may wait for its answer, and
 * how the channel tells "a slow answer" from "a dead link".
 *
 * A call waits as long as it says it may: an explicit budget (the
 * `X-Linggen-Budget-Ms` header a page sets on its fetch), else a JSON body's
 * own `timeout_ms` (the work it asks the engine to run that long — /api/bash,
 * /api/bridge) plus a margin for the reply, else 30 s. A call past its budget
 * rejects alone; it says nothing about the link. Whether the link lives is the
 * heartbeat's to say (`createLiveness`): a 30 s cap that also redialled used
 * to kill every long `du` the moment it ran past 30 s (2026-11-03, Shifu's
 * full rescan). Pure, so node tests hold it. A skill page's
 * shared/channel.js only carries the header's value to its host (the UI's
 * channelHost.ts), which applies this policy.
 */

/** What a call that declares nothing may wait. */
export const DEFAULT_BUDGET_MS = 30_000;
/** Time for the reply on top of the work a body's `timeout_ms` asks for. */
export const REPLY_MARGIN_MS = 15_000;
/** No call waits longer than this, whatever it declares. */
export const MAX_BUDGET_MS = 2 * 60 * 60 * 1000;
/** The fetch header a page names its call's budget in, in milliseconds. */
export const BUDGET_HEADER = 'X-Linggen-Budget-Ms';

function positive(n: unknown): number | null {
  const v = typeof n === 'string' && n.trim() !== '' ? Number(n) : n;
  return typeof v === 'number' && Number.isFinite(v) && v > 0 ? v : null;
}

function clamp(ms: number): number {
  return Math.min(Math.max(ms, DEFAULT_BUDGET_MS), MAX_BUDGET_MS);
}

/** The `timeout_ms` a parsed JSON body asks the engine to run for, if any. */
function bodyTimeout(body: unknown): number | null {
  if (!body || typeof body !== 'object') return null;
  return positive((body as { timeout_ms?: unknown }).timeout_ms);
}

/** One call's budget: the explicit one, else its body's `timeout_ms` plus the
 *  reply margin, else the default — never under the default nor over the cap. */
export function budgetMs(explicit: unknown, body?: unknown): number {
  const own = positive(explicit);
  if (own !== null) return clamp(own);
  const work = bodyTimeout(body);
  if (work !== null) return clamp(work + REPLY_MARGIN_MS);
  return DEFAULT_BUDGET_MS;
}

/** How often the channel pings the engine. */
export const PING_EVERY_MS = 15_000;
/** A ping unanswered this long means the link is dead. */
export const PONG_WAIT_MS = 30_000;

/** The heartbeat's verdict on the link. Dead only when a ping went out
 *  `PONG_WAIT_MS` ago and nothing at all came back since it was sent —
 *  never "nothing heard for N s", which a throttled hidden tab (timers once
 *  a minute) would read as dead while its link is fine. */
export interface Liveness {
  /** Anything arrived on the control channel. */
  heard(): void;
  /** A ping went out at `now`. */
  pinged(now: number): void;
  /** Whether the link is dead at `now`. */
  dead(now: number): boolean;
}

export function createLiveness(waitMs = PONG_WAIT_MS): Liveness {
  let unansweredSince: number | null = null;
  return {
    heard() {
      unansweredSince = null;
    },
    pinged(now: number) {
      if (unansweredSince === null) unansweredSince = now;
    },
    dead(now: number) {
      return unansweredSince !== null && now - unansweredSince > waitMs;
    },
  };
}
