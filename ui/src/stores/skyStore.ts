/**
 * The live sky behind the shell: whether it is on (this browser's choice) and
 * the weather sense's view (`/api/senses/weather`, the engine's — the place,
 * automatic or typed, and the last reading).
 */
import { useEffect, useMemo } from 'react';
import { create } from 'zustand';
import { useShallow } from 'zustand/react/shallow';
import { ApiError, apiGet, apiPut } from '../lib/api';
import { composeSky, parseForce, parseOverrides, type Sky, type SkyOverrides } from '../lib/skyChoice.mts';
import { backdropOf, parseYmd, todayOf, ymdOf, type Today } from '../lib/skyArt.mts';

export interface WeatherView {
  city: { name: string; country?: string; country_code?: string; latitude?: number; longitude?: number; auto?: boolean } | null;
  auto: boolean;
  off: boolean;
  reading: {
    kind: string; code: number; temp_c: number; is_day: boolean; city: string; at: number;
    wind_kmh?: number; gust_kmh?: number; wind_dir?: number; latitude?: number; country_code?: string;
  } | null;
}

const ON_KEY = 'linggen:sky';
const FORCE_KEY = 'linggen:sky-force';

const stored = (key: string): string | null => {
  try { return localStorage.getItem(key); } catch { return null; }
};

/** `?sky=snow-night` or localStorage `linggen:sky-force` — to see a sky the day doesn't have. */
const forced = (): Sky | null =>
  parseForce(new URLSearchParams(window.location.search).get('sky') ?? stored(FORCE_KEY));

/** `&wind=…&season=…&festival=…&date=…` — dev overrides beside `?sky=`. */
const overrides = (): SkyOverrides => parseOverrides(window.location.search);

interface SkyState {
  enabled: boolean;
  view: WeatherView | null;
  force: Sky | null;
  overrides: SkyOverrides;
  /** The calendar's day (festival, 节气, season), recomputed on each load. */
  today: Today | null;
  error: string | null;
  setEnabled: (on: boolean) => void;
  load: () => Promise<void>;
  /** A typed city, or null for automatic. */
  setCity: (name: string | null) => Promise<void>;
  setOff: (off: boolean) => Promise<void>;
}

export const useSkyStore = create<SkyState>((set) => {
  const put = async (body: Record<string, unknown>) => {
    try {
      set({ view: await apiPut<WeatherView>('/api/senses/weather', body), error: null });
    } catch (e) {
      const notFound = e instanceof ApiError && e.status === 404;
      set({ error: notFound ? `No place named “${body.city}”.` : String(e) });
    }
  };
  return {
    enabled: stored(ON_KEY) !== 'off',
    view: null,
    force: forced(),
    overrides: overrides(),
    today: null,
    error: null,
    setEnabled: (on) => {
      try { localStorage.setItem(ON_KEY, on ? 'on' : 'off'); } catch { /* this browser only */ }
      set({ enabled: on });
    },
    load: async () => {
      try { set({ view: await apiGet<WeatherView>('/api/senses/weather') }); } catch { /* keep the last */ }
      set((s) => ({ today: todayFor(s.view, s.overrides) }));
    },
    setCity: (name) => put({ city: name }),
    setOff: (off) => put({ off }),
  };
});

/** The calendar's day where the person is: the city's latitude and country. */
function todayFor(view: WeatherView | null, o: SkyOverrides): Today {
  const day = parseYmd(o.date) ?? ymdOf(new Date());
  const latitude = view?.city?.latitude ?? view?.reading?.latitude ?? null;
  const country = view?.city?.country_code ?? view?.reading?.country_code ?? null;
  return todayOf(day, { latitude, country });
}

/** The sky to draw: the weather (forced or read), its wind, the season, the festival. */
export function useSky(): Sky | null {
  const { view, today, force, overrides: o } = useSkyStore(useShallow((s) => ({ view: s.view, today: s.today, force: s.force, overrides: s.overrides })));
  return useMemo(() => composeSky(view, today, force, o), [view, today, force, o]);
}

/** The faint picture behind the overlay: today's festival's, else the 节气's (or none). */
export const useSkyBackdrop = (): string | null =>
  useSkyStore((s) => (s.today ? backdropOf(s.today)?.src ?? null : null));

const POLL_MS = 10 * 60 * 1000;
let pollers = 0;
let pollTimer: ReturnType<typeof setInterval> | null = null;

/** Keep the weather view current while the page is visible — one poll for
 *  the page however many surfaces (the sky, the top-bar chip) ask. */
export function useWeatherPoll() {
  useEffect(() => {
    pollers += 1;
    if (pollers === 1) {
      useSkyStore.getState().load();
      pollTimer = setInterval(() => { if (!document.hidden) useSkyStore.getState().load(); }, POLL_MS);
    }
    return () => {
      pollers -= 1;
      if (pollers > 0 || !pollTimer) return;
      clearInterval(pollTimer);
      pollTimer = null;
    };
  }, []);
}
