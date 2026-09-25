// One app chat holds more than one agent's turns: the skill's own agent and a
// guest addressed by `@name` (Yinyue). A turn's end belongs to the agent that
// ran it — these pure rules keep one agent's TurnComplete from ending another's
// running turn, question or spinner. Pure, so node tests can hold them.

/** The run fields these rules read (a subset of AgentRunRecord). */
export interface RunRow {
  run_id: string;
  session_id: string;
  agent_id: string;
  parent_run_id: string | null;
  status: string;
  ended_at?: number | null;
}

const same = (a: string | null | undefined, b: string | null | undefined) =>
  !!a && !!b && a.toLowerCase() === b.toLowerCase();

/** This agent's top-level running rows in the session, marked completed;
 *  every other agent's rows as they were. */
export function completeAgentRuns<R extends RunRow>(runs: R[], sid: string, agentId: string, now: number): R[] {
  return runs.map((r) => {
    if (r.session_id !== sid || r.status !== 'running' || r.parent_run_id) return r;
    if (!same(r.agent_id, agentId)) return r;
    return { ...r, status: 'completed', ended_at: now };
  });
}

/** Whether another agent than this one still runs a top-level turn here. */
export function otherAgentRunning(runs: RunRow[], sid: string, agentId: string): boolean {
  return runs.some((r) => r.session_id === sid && r.status === 'running'
    && !r.parent_run_id && !same(r.agent_id, agentId));
}

/** Whether a turn's end closes the open question: only when the question is
 *  this agent's own, or one of its subagents'. An unowned question is ended
 *  by any turn, as before. */
export function askEndsWithTurn(
  askAgentId: string | null | undefined,
  agentId: string,
  parentOf: (id: string) => string | undefined,
): boolean {
  if (!askAgentId) return true;
  return same(askAgentId, agentId) || same(parentOf(askAgentId), agentId);
}

/** Whether a turn's end clears the optimistic "sent" flag: only when the send
 *  went to this agent (or its agent is unknown). */
export function sendEndsWithTurn(sentTo: string | null | undefined, agentId: string): boolean {
  return !sentTo || same(sentTo, agentId);
}

/** A chat row the stream relay reads. */
export interface RelayRow {
  role: string;
  from?: string;
  text?: string;
}

/** Whether a stream is the page's own turn: the skill's own agent's (any
 *  agent's when none is known), never a guest's and never a subagent's —
 *  a subagent's stream is its parent's tool call. */
export function ownsStream(agent: string, ownAgent: string, subagent = false): boolean {
  return !subagent && (!ownAgent || same(agent, ownAgent));
}

/** A token for the page; a guest's carries `own: false` so the bridge can
 *  keep it from the page's turn handlers. */
export function streamTokenPayload(agent: string, text: string, done: boolean, own: boolean) {
  return { text, done, agent, own };
}

/** The finished turn's words: the last row *this* agent wrote — never
 *  another agent's that happens to be last. */
export function streamEndPayload(agent: string, rows: RelayRow[], own: boolean) {
  const last = [...rows].reverse().find((m) => m.role === 'agent' && same(m.from, agent));
  return { text: last?.text || '', agent, own };
}

/** The agent a skill page's chat belongs to: the one the page pinned, else
 *  the engine's first addressable agent (its default). A page's hidden
 *  reports go here — never to whoever `@@name` last left the chat with. */
export function pageAgentOf(pinned: string, agents: { name: string; internal?: boolean }[]): string {
  if (pinned.trim()) return pinned.trim().toLowerCase();
  const first = agents.find((a) => !a.internal);
  return first ? first.name.toLowerCase() : '';
}

/** A send reply that says no turn ran (nothing kept, nothing will stream):
 *  the agent isn't in this app's world yet, or can't answer here. */
export function turnlessReply(status: string | undefined): status is 'absent' | 'unavailable' {
  return status === 'absent' || status === 'unavailable';
}
