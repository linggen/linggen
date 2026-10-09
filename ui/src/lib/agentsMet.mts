// Agents that appear only once met (engine `agent/met.rs`, page_state
// `agents_met`). The engine already leaves an unmet agent out of its lists;
// this is the surface's one reading of the same fact, so a page never shows
// an agent the engine says is not there yet.

export interface AgentMet {
  id: string;
  present: boolean;
  met: { at: string; reason: string } | null;
}

/** Her id: the agent whose surfaces (pet, voice, dock, settings) wait on it. */
export const COMPANION_ID = 'yinyue';

/** Whether `id` is there. Only agents that declare `met_when` are listed, so
 *  one that is not listed is always present. `met` null: no word from the
 *  engine yet — a met-gated agent stays out of sight until it speaks, which
 *  keeps a fresh install from flashing her for the first moment. */
export function isAgentPresent(met: readonly AgentMet[] | null | undefined, id: string): boolean {
  if (!met) return id.toLowerCase() !== COMPANION_ID;
  const row = met.find((m) => m.id.toLowerCase() === id.toLowerCase());
  return row ? row.present : true;
}

/** `agents` less those the engine says are not met. */
export function presentAgents<T extends { name: string }>(
  agents: readonly T[],
  met: readonly AgentMet[] | null | undefined,
): T[] {
  return agents.filter((a) => isAgentPresent(met, a.name));
}
