/**
 * How the shell wears the live sky — pure, so it is tested without a page.
 * `glass`: the sky moves behind frosted panels; `solid`: a still sky with
 * opaque panels (reduced motion, no backdrop-filter, a slow machine); null:
 * no sky (turned off in Settings, or no weather known).
 */
export type SkyMode = 'glass' | 'solid' | null;

export interface Sky {
  kind: string;
  is_day: boolean;
}

export const SKY_KINDS = ['clear', 'cloudy', 'fog', 'rain', 'snow', 'storm'];

/** `rain`, `snow-night`, … — the dev override (`?sky=` or localStorage). */
export function parseForce(raw: string | null | undefined): Sky | null {
  const [kind, time] = (raw ?? '').trim().toLowerCase().split('-');
  if (!SKY_KINDS.includes(kind)) return null;
  return { kind, is_day: time !== 'night' };
}

/** The reading of a `/api/senses/weather` view as a sky. */
export function skyOfView(view: { reading?: { kind?: string; is_day?: boolean } | null } | null): Sky | null {
  const r = view?.reading;
  if (!r?.kind || !SKY_KINDS.includes(r.kind)) return null;
  return { kind: r.kind, is_day: r.is_day !== false };
}

export function skyMode(o: { enabled: boolean; sky: Sky | null; frost: boolean; still: boolean }): SkyMode {
  if (!o.enabled || !o.sky) return null;
  if (o.still || !o.frost) return 'solid';
  return 'glass';
}

/** The place line in Settings: "Halifax, CA · automatic". */
export function placeLine(view: { city?: { name: string; country?: string } | null; auto?: boolean; off?: boolean } | null): string {
  if (!view) return '…';
  if (view.off) return 'Weather is off';
  const where = view.city ? [view.city.name, view.city.country].filter(Boolean).join(', ') : 'Finding your city…';
  return view.auto ? `${where} · automatic` : where;
}
