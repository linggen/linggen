/**
 * Settings → Tools & MCP → Outside agents: the private apps an agent outside
 * Linggen (Claude Code, Codex) may use, as the engine answers them
 * (`GET /api/mcp/outside-apps`, `server/api/outside_apps.rs`). The engine says
 * which apps may be opened and which are; the page lists them and sends the
 * whole list of open ones when one switch moves.
 *
 * Every sentence the section shows is in `WORDS`: twenty words in all
 * (tests/outsideApps.test.mts holds that, and a Chinese entry for each).
 */

/** A private app that offers an action anywhere, as the engine lists it. */
export interface OutsideApp {
  name: string;
  description: string;
  open: boolean;
}

export interface OutsideAppsReply {
  apps: OutsideApp[];
  /** The one who asks is at this Mac itself — only there is the list set. */
  at_this_mac: boolean;
}

/** Everything the section says. */
export const WORDS = {
  title: 'Outside agents',
  hint: 'Private apps stay closed to them until you open one.',
  switch: 'Outside agents may use it',
};

/** The names of the open apps once `name` is switched to `on`. */
export function openAfterToggle(apps: readonly OutsideApp[], name: string, on: boolean): string[] {
  return apps.filter((a) => (a.name === name ? on : a.open)).map((a) => a.name);
}
