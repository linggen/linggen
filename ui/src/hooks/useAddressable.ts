import { useMemo } from 'react';
import type { AgentInfo } from '../types';
import { COMPANION_ID } from '../lib/agentsMet.mts';
import { useAgentPresent } from './useAgentPresent';

/** The agents a person may address in this chat: its main agents, less any
 *  the spec marks `internal`, and the companion only once she is there. The
 *  `@` list, the box's addressee chip and the history's `→ name` all read
 *  this one list. */
export function useAddressable(mainAgents: AgentInfo[]): AgentInfo[] {
  const companionHere = useAgentPresent(COMPANION_ID);
  return useMemo(
    () => mainAgents.filter((a) => !a.internal && (a.name.toLowerCase() !== COMPANION_ID || companionHere)),
    [mainAgents, companionHere],
  );
}
