import { useServerStore } from '../stores/serverStore';
import { isAgentPresent } from '../lib/agentsMet.mts';

/** Whether `id` is there yet (page_state `agents_met`). Re-renders the moment
 *  the engine says someone is newly met, so their surfaces appear without a
 *  reload. */
export function useAgentPresent(id: string): boolean {
  return useServerStore((s) => isAgentPresent(s.agentsMet, id));
}
