// The chat input's mention grammar, as pure functions:
//   @agent   — address a main agent by id or declared alias (`@银月 …`), for
//              this message only (only at the start of a message)
//   @@agent  — the same, and the chat stays with that agent
//   /f name  — pick a file: `/f src/` browses a directory, `/f name` searches;
//              the chosen file is written into the message as `@path`
// The server reads the same grammar (chat/handler.rs leading_mention — the longest id or alias at the start),
// so a page that sends `@银月 …` through the embed reaches her either way.

/** A main agent and the names it answers to besides its id. */
export interface MentionableAgent {
  name: string;
  aliases?: string[];
}

/** What may close an `@name` besides whitespace (matches the server). */
const NAME_END = /^[\s,:，：、]/;
const NAME_ENDS = /^[\s,:，：、]+/;

/** Whether `rest` opens with `name` as a whole name: what follows is the end,
 *  a closing mark, or — when either side of the seam is CJK — anything
 *  (`@银月你好`). `@lingo` is never `@ling`. */
function opensWithName(rest: string, name: string): boolean {
  if (!name || rest.slice(0, name.length).toLowerCase() !== name.toLowerCase()) return false;
  const next = rest.charAt(name.length);
  if (!next || NAME_END.test(next)) return true;
  return CJK.test(next) || CJK.test(name.charAt(name.length - 1));
}

/** The agent a message opens by addressing, if any: its id, whether the chat
 *  should stay with it (`@@`), and the words after the name. The name ends
 *  at the longest id or alias the text starts with — the server reads the
 *  same rule. A `@path` never matches — a path is not an agent's name. */
export function leadingAgentMention(
  text: string,
  agents: MentionableAgent[],
): { agent: string; sticky: boolean; body: string } | undefined {
  const t = text.trim();
  if (!t.startsWith('@')) return undefined;
  const sticky = t.startsWith('@@');
  const rest = t.slice(sticky ? 2 : 1);
  let best: { agent: string; len: number } | undefined;
  for (const a of agents) {
    for (const n of [a.name, ...(a.aliases ?? [])].map((x) => x.trim())) {
      if (n.length > (best?.len ?? 0) && opensWithName(rest, n)) best = { agent: a.name.toLowerCase(), len: n.length };
    }
  }
  if (!best) return undefined;
  const body = rest.slice(best.len).replace(NAME_ENDS, '');
  return body ? { agent: best.agent, sticky, body } : undefined;
}

/** Whether the text is only an agent's name after `@` / `@@` — an address
 *  with nothing said yet (the pre-filled box, sent as it stands). */
export function isBareMention(text: string, agents: MentionableAgent[]): boolean {
  const t = text.trim();
  if (!t.startsWith('@')) return false;
  const name = t.replace(/^@@?/, '').replace(NAME_ENDS, '').toLowerCase();
  return agents.some((a) => [a.name, ...(a.aliases ?? [])].some((n) => n.trim().toLowerCase() === name));
}

/** The input with its trailing `@@partial` completed to `@@Agent `. */
export function completeAgentMention(input: string, agentName: string): string {
  const at = input.lastIndexOf('@@');
  const before = at >= 0 ? input.substring(0, at) : input;
  return `${before}@@${agentName.charAt(0).toUpperCase() + agentName.slice(1)} `;
}

/** The file picker's token at the end of the input: `/f` or `/f query`. */
const FILE_TOKEN = /(^|\s)\/f(?:\s(\S*))?$/;

/** The input with its trailing `/f query` token replaced by `@path` (plus a
 *  space when the reference is complete, i.e. a file); a directory keeps the
 *  picker open as `/f dir/`. */
export function completeFileMention(input: string, path: string, complete: boolean): string {
  return input.replace(FILE_TOKEN, (_m, pre: string) => (complete ? `${pre}@${path} ` : `${pre}/f ${path}`));
}

export type MentionInProgress =
  | { kind: 'agent'; filter: string }
  | { kind: 'lead-agent'; filter: string }
  | { kind: 'file-browse'; dir: string; filter: string }
  | { kind: 'file-search'; query: string };

/** The mention being typed at the end of the input, if any: `@@x` (stay with
 *  an agent), a leading `@x` (address an agent), or `/f x` (pick a file). */
export function mentionInProgress(val: string): MentionInProgress | null {
  const f = FILE_TOKEN.exec(val);
  if (f) {
    const after = f[2] ?? '';
    const slash = after.lastIndexOf('/');
    if (slash >= 0) return { kind: 'file-browse', dir: after.substring(0, slash + 1), filter: after.substring(slash + 1) };
    return { kind: 'file-search', query: after };
  }
  const lastAt = val.lastIndexOf('@');
  if (lastAt < 0 || val.includes(' ', lastAt)) return null;
  const after = val.substring(lastAt + 1);
  if (lastAt > 0 && val[lastAt - 1] === '@') return { kind: 'agent', filter: after.toLowerCase() };
  if (after.startsWith('@') || val.substring(0, lastAt).trim() !== '') return null;
  return { kind: 'lead-agent', filter: after };
}

/** Whether `filter` (what follows a leading `@`) could name this agent: a
 *  prefix-free substring of its id or any alias. Plain `includes`, so CJK
 *  (`@银` → 银月) matches as well as Latin (`@yin` → yinyue). */
export function agentMatches(agent: MentionableAgent, filter: string): boolean {
  const f = filter.trim().toLowerCase();
  if (!f) return true;
  return [agent.name, ...(agent.aliases ?? [])].some((n) => n.trim().toLowerCase().includes(f));
}

/** CJK script: a name in it needs no space before the words (`@银月你好`). */
const CJK = /[\u3040-\u30ff\u3400-\u9fff\uf900-\ufaff\uac00-\ud7af]/;

/** The language the page speaks: the framing page's `<html lang>` (a skill
 *  page such as a zh game, served from our own origin), else the browser's.
 *  Our own `<html lang>` is fixed in index.html, so it says nothing. Empty
 *  when neither is known. */
export function mentionLanguage(): string {
  try {
    if (window.parent !== window) {
      const lang = window.parent.document.documentElement.lang;
      if (lang) return lang;
    }
  } catch {
    /* a parent on another origin keeps its language to itself */
  }
  return typeof navigator !== 'undefined' ? navigator.language || '' : '';
}

/** The name to write after `@` for this agent in the page's language: a CJK
 *  alias when the language is CJK — or unknown — and the agent has one, else
 *  its id, capitalized. (The dropdown shows the other names beside it, so an
 *  unknown language reads `@银月 · Yinyue`.) */
export function agentMentionLabel(agent: MentionableAgent, lang: string): string {
  const cjkLang = !lang.trim() || /^(zh|ja|ko)/i.test(lang);
  const alias = cjkLang ? (agent.aliases ?? []).find((a) => CJK.test(a)) : undefined;
  return alias?.trim() || agent.name.charAt(0).toUpperCase() + agent.name.slice(1);
}

/** The input with its leading `@partial` replaced by `@Label `. */
export function completeLeadingAgentMention(input: string, label: string): string {
  return `${input.substring(0, input.lastIndexOf('@'))}@${label} `;
}

// The composer's pre-fill (one-ling-spec § Who answers; the phone's
// screens/chat/dispatch.dart is the reference): once the user addresses
// Yinyue, or she replies, the box opens with her mention so the talk stays
// with her. Deleting it hands the next line back to Ling. It clears after
// PREFILL_QUIET_MS without a word from her.

/** How long the pre-filled mention outlives the last exchange with her. */
export const PREFILL_QUIET_MS = 10 * 60 * 1000;

/** The slice of a chat row the table reads. */
export interface TableMessage {
  role: 'user' | 'agent';
  from?: string;
  to?: string;
  timestampMs?: number;
}

/** Who held the table last, and when. */
export interface Floor {
  speaker?: string;
  at?: number;
}

/** The last exchange at the table: a user line counts for the agent it was
 *  sent to, a reply for the agent that wrote it. Rows between anyone else
 *  (`system`, shell lines, sub-agents) are not at the table. */
export function floorOf(messages: TableMessage[], tableIds: string[]): Floor {
  const at = new Set(tableIds.map((id) => id.toLowerCase()));
  for (let i = messages.length - 1; i >= 0; i--) {
    const m = messages[i];
    const speaker = (m.role === 'agent' ? m.from : m.to)?.toLowerCase();
    if (speaker && at.has(speaker)) return { speaker, at: m.timestampMs };
  }
  return {};
}

/** The box's pre-fill: `@Label ` while Yinyue holds the floor and it is
 *  fresher than PREFILL_QUIET_MS, else empty. */
export function prefillAfter(floor: Floor, now: number, label: string): string {
  if (floor.speaker !== 'yinyue' || floor.at === undefined) return '';
  return now - floor.at >= PREFILL_QUIET_MS ? '' : `@${label} `;
}
