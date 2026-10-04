// A session's members — the agents at its table (doc/shared-session-spec.md).
// The server keeps them on the session (`agents`, the lead first) and decides
// who answers; the surface mirrors the same rules to show who that is.

export interface SessionMember {
  id: string;
  model?: string | null;
}

/** The agent an ordinary chat seats, and who answers on the Mac when a
 *  message names nobody. Mirrors `LEAD_AGENT_ID` in the engine. */
export const LEAD_AGENT = 'ling';

/** Who answers a message that names nobody: Ling when seated, else the
 *  session's first member (`members::default_responder`). */
export function defaultResponder(members: readonly SessionMember[] | null | undefined): string {
  const list = members ?? [];
  if (list.some((m) => m.id === LEAD_AGENT)) return LEAD_AGENT;
  return list[0]?.id ?? LEAD_AGENT;
}

/** The agent this chat speaks with: the member the person picked, else the
 *  default responder. */
export function chatAgentOf(
  picked: string | null | undefined,
  members: readonly SessionMember[] | null | undefined,
): string {
  const p = (picked ?? '').trim().toLowerCase();
  return p || defaultResponder(members);
}

/** The model the session keeps for `agent`, if any. */
export function memberModel(
  members: readonly SessionMember[] | null | undefined,
  agent: string,
): string | null {
  return (members ?? []).find((m) => m.id === agent)?.model ?? null;
}

/** `members` with `agent`'s model set (seating it when it isn't there). */
export function withMemberModel(
  members: readonly SessionMember[] | null | undefined,
  agent: string,
  model: string | null,
): SessionMember[] {
  const list = [...(members ?? [])];
  const at = list.findIndex((m) => m.id === agent);
  if (at < 0) return [...list, { id: agent, model }];
  list[at] = { ...list[at], model };
  return list;
}
