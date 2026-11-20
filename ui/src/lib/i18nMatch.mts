/**
 * One piece of the page's English found in a string table: the exact text,
 * else the first entry whose `{slot}` pattern fits (a slot matches any run of
 * text and carries over). lib/i18n.ts says it on the page; a test says it for
 * every sentence a component builds.
 */
type Pattern = { re: RegExp; names: string[]; zh: string };

const indexed = new WeakMap<Record<string, string>, Pattern[]>();

function index(table: Record<string, string>): Pattern[] {
  const patterns: Pattern[] = [];
  for (const [en, zh] of Object.entries(table)) {
    const names = [...en.matchAll(/\{(\w+)\}/g)].map((m) => m[1]);
    if (!names.length) continue;
    const body = en
      .split(/\{\w+\}/)
      .map((s) => s.replace(/[.*+?^${}()|[\]\\]/g, '\\$&'))
      .join('(.+?)');
    patterns.push({ re: new RegExp(`^${body}$`), names, zh });
  }
  return patterns;
}

/** The table's words for `text`, or null. */
export function translateFrom(table: Record<string, string>, text: string): string | null {
  const s = text.trim();
  if (!s) return null;
  const hit = table[s];
  if (hit !== undefined) return hit;
  let patterns = indexed.get(table);
  if (!patterns) indexed.set(table, (patterns = index(table)));
  for (const p of patterns) {
    const m = p.re.exec(s);
    if (m) return p.zh.replace(/\{(\w+)\}/g, (_, k: string) => m[p.names.indexOf(k) + 1] ?? `{${k}}`);
  }
  return null;
}
