/** A Claude session from 2.5.1 and before (`runtime: claude_agent_sdk`): the
 *  SDK that ran it is gone, so the page shows it read-only and offers to go
 *  on in a new Linggen session (POST /api/sessions/continue). The one place
 *  the page tells such a session apart — tests/oldClaude.test.mts fails when
 *  the runtime's name appears anywhere else. */
export const isOldClaude = (session?: { runtime?: string } | null): boolean =>
  session?.runtime === 'claude_agent_sdk';

/** The line the page shows in place of the composer (≤ 20 words). */
export const OLD_CLAUDE_LINE = 'Claude sessions now run as Linggen sessions, so this one is read-only.';

/** What the page says once the new session is made: the model it runs on (≤ 20 words). */
export const continuedLine = (model: string): string => `Continuing in a new session on ${model}.`;
