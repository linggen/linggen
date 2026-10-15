/**
 * The shell's sky, pure so it is tested without a page: which weather to
 * draw (with its wind, the season and the day's festival), and the place
 * line Settings shows.
 */
import { FESTIVALS, windNamed, windOf, type Today, type Wind } from './skyArt.mts';
import { SEASONS } from './lunar.mts';

export interface Sky {
  /** A weather kind, or `none` (no reading: only the season or festival draws). */
  kind: string;
  is_day: boolean;
  wind?: Wind | null;
  season?: string | null;
  /** The festival's particle kind for the day, if any. */
  festival?: string | null;
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

/** The place line in Settings: "Halifax, CA · automatic". */
export function placeLine(view: { city?: { name: string; country?: string } | null; auto?: boolean; off?: boolean } | null): string {
  if (!view) return '…';
  if (view.off) return 'Weather is off';
  const where = view.city ? [view.city.name, view.city.country].filter(Boolean).join(', ') : 'Finding your city…';
  return view.auto ? `${where} · automatic` : where;
}

/** Dev overrides beside `?sky=`: `&wind=gale&winddir=270&season=autumn&festival=zhongqiu&date=2026-10-31`. */
export interface SkyOverrides { wind?: Wind; season?: string; festival?: string; date?: string }

export function parseOverrides(search: string): SkyOverrides {
  const q = new URLSearchParams(search);
  const dir = q.get('winddir');
  const wind = windNamed(q.get('wind'), dir != null ? Number(dir) : undefined);
  const season = q.get('season');
  const festival = q.get('festival');
  return {
    ...(wind ? { wind } : {}),
    ...(season && (SEASONS as readonly string[]).includes(season) ? { season } : {}),
    ...(festival && FESTIVALS[festival] ? { festival } : {}),
    ...(q.get('date') ? { date: q.get('date') as string } : {}),
  };
}

type Reading = { kind?: string; is_day?: boolean; wind_kmh?: number | null; gust_kmh?: number | null; wind_dir?: number | null } | null;

/** The whole sky: the weather (forced or read), its wind, the season and the
 *  festival's particles. Null when there is nothing at all to draw. */
export function composeSky(view: { reading?: Reading; city?: unknown; off?: boolean } | null, today: Today | null, force: Sky | null, o: SkyOverrides = {}): Sky | null {
  const weather = force ?? skyOfView(view);
  const festivalId = o.festival ?? today?.festival?.id ?? null;
  const festival = festivalId ? FESTIVALS[festivalId]?.particles ?? null : null;
  // The season needs a place (its hemisphere): none while the sense is off or has no city.
  const season = o.season ?? (view?.city && !view.off ? today?.season ?? null : null);
  const wind = o.wind ?? windOf(view?.reading ?? null);
  if (!weather && !festival && !season) return null;
  return { kind: weather?.kind ?? 'none', is_day: weather?.is_day ?? true, wind, season, festival };
}
