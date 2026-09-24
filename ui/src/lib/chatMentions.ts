// The chat input's mention grammar, as pure functions:
//   @agent   — address a main agent by id or declared alias (`@银月 …`), for
//              this message only (only at the start of a message)
//   @@agent  — the same, and the chat stays with that agent
//   @path    — a file: `@src/` browses a directory, `@name` searches
// The server reads the same grammar (chat/handler.rs parse_explicit_target_prefix),
// so a page that sends `@银月 …` through the embed reaches her either way.

/** A main agent and the names it answers to besides its id. */
export interface MentionableAgent {
  name: string;
  aliases?: string[];
}

/** What may close an `@name` besides whitespace (matches the server). */
const NAME_END = /[\s,:，：、]/;

/** The agent a message opens by addressing, if any: its id, and whether the
 *  chat should stay with it (`@@`). A `@path` never matches — a path is not
 *  an agent's name. */
export function leadingAgentMention(
  text: string,
  agents: MentionableAgent[],
): { agent: string; sticky: boolean } | undefined {
  const t = text.trim();
  if (!t.startsWith('@')) return undefined;
  const sticky = t.startsWith('@@');
  const rest = t.slice(sticky ? 2 : 1);
  const end = rest.search(NAME_END);
  if (end <= 0 || !rest.slice(end).replace(/^[\s,:，：、]+/, '')) return undefined;
  const name = rest.slice(0, end).toLowerCase();
  const hit = agents.find((a) =>
    a.name.toLowerCase() === name || (a.aliases ?? []).some((al) => al.trim().toLowerCase() === name));
  return hit ? { agent: hit.name.toLowerCase(), sticky } : undefined;
}

/** The input with its trailing `@@partial` completed to `@@Agent `. */
export function completeAgentMention(input: string, agentName: string): string {
  const at = input.lastIndexOf('@@');
  const before = at >= 0 ? input.substring(0, at) : input;
  return `${before}@@${agentName.charAt(0).toUpperCase() + agentName.slice(1)} `;
}

/** The input with its trailing `@partial` replaced by `@path` (plus a space
 *  when the mention is complete, i.e. a file rather than a directory). */
export function completeFileMention(input: string, path: string, complete: boolean): string {
  return `${input.substring(0, input.lastIndexOf('@'))}@${path}${complete ? ' ' : ''}`;
}

export type MentionInProgress =
  | { kind: 'agent'; filter: string }
  | { kind: 'file-browse'; dir: string; filter: string }
  | { kind: 'file-search'; query: string };

/** The mention being typed at the end of the input, if any. */
export function mentionInProgress(val: string): MentionInProgress | null {
  const lastAt = val.lastIndexOf('@');
  if (lastAt < 0 || val.includes(' ', lastAt)) return null;
  const after = val.substring(lastAt + 1);
  if (lastAt > 0 && val[lastAt - 1] === '@') return { kind: 'agent', filter: after.toLowerCase() };
  if (after.startsWith('@')) return null;
  const slash = after.lastIndexOf('/');
  if (slash >= 0) return { kind: 'file-browse', dir: after.substring(0, slash + 1), filter: after.substring(slash + 1) };
  return { kind: 'file-search', query: after };
}
