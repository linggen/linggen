// The agent a skill page's chat belongs to — the skill's own agent. A guest
// (`@银月`) answers in the same chat, but the page's hidden reports and its
// turn handlers are for this agent alone.
import { useServerStore } from '../stores/serverStore';
import { pageAgentOf } from './agentTurns.mts';

let pinned = '';

/** The embed's `?agent=` (the page's `mount({ agentId })`). */
export function setPageAgent(agentId: string): void {
  pinned = agentId;
}

/** The pinned agent, else the engine's first addressable agent (its default,
 *  as the chat auto-selects on a fresh client). Empty before agents load. */
export function pageAgent(): string {
  return pageAgentOf(pinned, useServerStore.getState().agents);
}
