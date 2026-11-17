// The offers her line carries — buttons under her label, each running one
// declared thing as the person's own tap (`doc/perception-spec.md` §5). A
// tool runs through the skill-tool route a page button uses; a place opens
// through the app launch route. Neither agent acts here.
//
// Her labels ride her reply as `[offer N: label]` markers; the engine takes
// them out before her voice (`resident::offer_labels`) and the thread takes
// them out here, keeping the labels as the row's record of what she offered.

export type OfferButtonLike = {
  label: string;
  kind: string;
  skill: string;
  tool: string | null;
  args: Record<string, unknown> | null;
  tier: string | null;
  view: string | null;
};

const MARKER = /\s*\[offer \d+:\s*([^\]]{1,40})\]/g;

/** Her reply without its offer markers, and the labels they carried. */
export function stripOfferMarkers(text: string): { text: string; labels: string[] } {
  const labels: string[] = [];
  const rest = text.replace(MARKER, (_m, label: string) => {
    labels.push(label.trim());
    return '';
  });
  return { text: rest.trim(), labels };
}

/** A tool that changes anything asks before it runs; a read and a place do not. */
export function needsConfirm(o: OfferButtonLike): boolean {
  return o.kind === 'tool' && o.tier !== 'read';
}

/** The one request a tap sends. */
export function offerRequest(o: OfferButtonLike): { url: string; body: unknown } {
  if (o.kind === 'tool' && o.tool) {
    const url = `/api/skills/${encodeURIComponent(o.skill)}/tools/${encodeURIComponent(o.tool)}`;
    return { url, body: o.args ?? {} };
  }
  return { url: '/api/apps/launch', body: { skill: o.skill, view: o.view ?? undefined } };
}

/** What a tap came to, from the route's answer: a tool's own exit code. */
export function outcomeOf(o: OfferButtonLike, ok: boolean, result: unknown): 'done' | 'failed' {
  if (!ok) return 'failed';
  if (o.kind !== 'tool') return 'done';
  const code = (result as { exit_code?: number } | null)?.exit_code;
  return code === undefined || code === 0 ? 'done' : 'failed';
}
