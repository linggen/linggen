// What a Bash call reads as in the chat: its tool row (`Bash(<label>)`) and
// its line in the turn's activity log ("Running command: <label>").
// Both runtimes reach the UI the same way — a tool_use block with the call's
// args — so this is the one place the label is made.
//
// - The call says what it does: its `description` (Claude Code's Bash has
//   one; native Bash takes one too). That is the label.
// - Otherwise the command, made short: the `cd <dir> &&` and env set-up in
//   front and the `2>&1 | tail -N` behind are plumbing, not the work; a home
//   path is `~/…`, a deep one its last part.
//
// The full command stays on the block's args; the expanded row shows it.

/** The longest label shown, in characters. */
export const BASH_LABEL_MAX = 80;

// One shell word: quoted strings and bare runs, glued together
// (`PATH="$HOME/bin:$PATH"` is one word).
const WORD = `(?:"(?:[^"\\\\]|\\\\.)*"|'[^']*'|[^\\s;&|"'])+`;
const SEP = `\\s*(?:&&|;)\\s*`;
const LEADING = [
  new RegExp(`^cd(?:\\s+${WORD})?${SEP}`),
  new RegExp(`^export(?:\\s+${WORD})+${SEP}`),
  new RegExp(`^(?:[A-Za-z_][A-Za-z0-9_]*=(?:${WORD})?\\s+)+`),
];
const TRAILING = [
  /\s*2>&1\s*$/,
  /\s*\|\s*(?:tail|head)(?:\s+-n)?(?:\s+-?\d+|\s*-\d+)?\s*$/,
];

// An absolute path, or one off $HOME / ~ — at a word's start, so the
// inside of a relative path (`node_modules/vite/bin`) is left alone.
const PATH_RE = /(?<![^\s"'=:(])(?:\$HOME|\$\{HOME\}|~|\/[^\s/"'`;|&<>()]+)(?:\/[^\s/"'`;|&<>()]*)+/g;
const HOME_RE = /^(?:\$HOME|\$\{HOME\}|~|\/Users\/[^/]+|\/home\/[^/]+)(?=\/)/;

/** A path as the label shows it: under home `~/a/b`, deeper its last part;
 *  elsewhere, a path three or more deep is its last part (`/opt/x/bin/node`
 *  is `node`; `/dev/null` stays). */
export function shortPath(path: string): string {
  const home = path.match(HOME_RE);
  const rest = home ? path.slice(home[0].length) : path;
  const parts = rest.split('/').filter(Boolean);
  if (parts.length === 0) return home ? '~' : path;
  if (home) return parts.length <= 2 ? `~/${parts.join('/')}` : parts[parts.length - 1];
  return parts.length >= 3 ? parts[parts.length - 1] : path;
}

function cap(text: string, max: number): string {
  return text.length <= max ? text : text.slice(0, max - 1).trimEnd() + '…';
}

function stripEnds(cmd: string): string {
  let s = cmd;
  for (let changed = true; changed;) {
    changed = false;
    for (const re of LEADING) {
      const next = s.replace(re, '');
      if (next !== s && next.trim()) { s = next; changed = true; }
    }
    for (const re of TRAILING) {
      const next = s.replace(re, '');
      if (next !== s && next.trim()) { s = next; changed = true; }
    }
  }
  return s;
}

/** A shell command made readable: set-up and output plumbing dropped, paths
 *  shortened, capped at `max`. */
export function shortCommand(cmd: string, max = BASH_LABEL_MAX): string {
  const flat = cmd.replace(/\s+/g, ' ').trim();
  if (!flat) return '';
  const core = stripEnds(flat).trim();
  return cap(core.replace(PATH_RE, shortPath), max);
}

/** A Bash call's label from its args: its description, else its command
 *  made short. Empty when it has neither. */
export function bashLabel(args: unknown, max = BASH_LABEL_MAX): string {
  const a = (args && typeof args === 'object' ? args : {}) as Record<string, unknown>;
  const description = typeof a.description === 'string' ? a.description.replace(/\s+/g, ' ').trim() : '';
  if (description) return cap(description, max);
  const cmd = typeof a.command === 'string' ? a.command : typeof a.cmd === 'string' ? a.cmd : '';
  return shortCommand(cmd, max);
}

/** The command a Bash call runs, whole — what its expanded row shows. */
export function bashCommand(args: unknown): string {
  const a = (args && typeof args === 'object' ? args : {}) as Record<string, unknown>;
  const cmd = typeof a.command === 'string' ? a.command : typeof a.cmd === 'string' ? a.cmd : '';
  return cmd.trim();
}
