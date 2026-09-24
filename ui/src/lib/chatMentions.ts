// The chat input's mention grammar, as pure functions:
//   @@agent  — address a main agent (only at the start of a message)
//   @path    — a file: `@src/` browses a directory, `@name` searches

/** The main agent a message opens with (`@@ling …`), if it names one. */
export function leadingAgentMention(text: string, mainAgentIds: string[]): string | undefined {
  const name = text.trim().match(/^@@([a-zA-Z0-9_-]+)\b/)?.[1]?.toLowerCase();
  return name && mainAgentIds.includes(name) ? name : undefined;
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
