// A session's status text (serverStore.agentStatusText), which the spinner
// under the chat shows: "✶ Working… (12s · 30 tok/s · …)". It says what
// state the agent is in, never which tool runs or what it runs; the tool row
// in the chat shows that, as Claude Code does.

/** The status a session has while one of its tools runs. */
export const WORKING = 'Working';

/** The status text for a top-level agent status event: the engine's own
 *  words for model loading, thinking and the rest ("Thinking (model)",
 *  "Compacting conversation"), but plain `Working` while a tool runs, whatever
 *  the event says about the tool. */
export function sessionStatusText(status: string, text: string): string {
  if (status === 'idle') return 'Idle';
  if (status === 'calling_tool') return WORKING;
  if (text) return text;
  switch (status) {
    case 'model_loading': return 'Model Loading';
    case 'thinking': return 'Thinking';
    case 'working': return WORKING;
    default: return 'Idle';
  }
}

/** What the spinner says for a session's status text. A spinner turns only
 *  while a turn runs, so a leftover `Idle` (the last turn's end, before the
 *  new turn's first status arrives) says nothing: the caller's verb shows. */
export function spinnerLabel(text: string | undefined): string | undefined {
  return text && text !== 'Idle' ? text : undefined;
}
