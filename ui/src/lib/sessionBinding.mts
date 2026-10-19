// The values an app page's chat is bound to (a skill's declared session
// params, e.g. a test save): `mount(el, {params})` in /shared/chat-bridge.js
// hands them to the embed as `?params=<json>`. Every session made through that
// embed carries them, and its list shows only sessions bound the same way —
// a page on a test save never starts or opens a chat on the main one. Pure,
// so node tests can hold it.

export type Binding = Record<string, string>;

/** The binding in the embed's `?params=`: a JSON object of words; anything
 *  else is no binding. Empty values are dropped, as `boundTo` drops them. */
export function parseBinding(raw: string | null | undefined): Binding {
  if (!raw) return {};
  let parsed: unknown;
  try { parsed = JSON.parse(raw); } catch { return {}; }
  if (!parsed || typeof parsed !== 'object' || Array.isArray(parsed)) return {};
  const out: Binding = {};
  for (const [k, v] of Object.entries(parsed as Record<string, unknown>)) {
    if (v == null || v === '') continue;
    if (typeof v === 'string' || typeof v === 'number' || typeof v === 'boolean') out[k] = String(v);
  }
  return out;
}

/** The sessions bound to exactly `params` — the rule of `boundTo` in
 *  /shared/api.js: the same names, the same values; `{}` keeps the unbound. */
export function boundTo<T extends { params?: Record<string, string> | null }>(
  sessions: T[],
  params: Binding = {},
): T[] {
  const want = Object.entries(params || {}).filter(([, v]) => v != null && v !== '');
  return (sessions || []).filter((s) => {
    const have = s.params || {};
    return Object.keys(have).length === want.length && want.every(([k, v]) => have[k] === String(v));
  });
}

/** The body of `POST /api/sessions` for a new chat: a skill session carries
 *  the binding; a plain chat never does (only a skill declares params). */
export function newChatRequest(
  title: string,
  skill?: string | null,
  params: Binding = {},
): { title: string; skill?: string; params?: Binding } {
  if (!skill) return { title };
  return Object.keys(params).length ? { title, skill, params } : { title, skill };
}
