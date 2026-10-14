/**
 * The live sky behind the shell: whether it is on (this browser's choice) and
 * the weather sense's view (`/api/senses/weather`, the engine's — the place,
 * automatic or typed, and the last reading).
 */
import { create } from 'zustand';
import { useShallow } from 'zustand/react/shallow';
import { ApiError, apiGet, apiPut } from '../lib/api';
import { parseForce, skyOfView, type Sky } from '../lib/skyChoice.mts';

export interface WeatherView {
  city: { name: string; country?: string; auto?: boolean } | null;
  auto: boolean;
  off: boolean;
  reading: { kind: string; code: number; temp_c: number; is_day: boolean; city: string; at: number } | null;
}

const ON_KEY = 'linggen:sky';
const FORCE_KEY = 'linggen:sky-force';

const stored = (key: string): string | null => {
  try { return localStorage.getItem(key); } catch { return null; }
};

/** `?sky=snow-night` or localStorage `linggen:sky-force` — to see a sky the day doesn't have. */
const forced = (): Sky | null =>
  parseForce(new URLSearchParams(window.location.search).get('sky') ?? stored(FORCE_KEY));

interface SkyState {
  enabled: boolean;
  view: WeatherView | null;
  force: Sky | null;
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
    error: null,
    setEnabled: (on) => {
      try { localStorage.setItem(ON_KEY, on ? 'on' : 'off'); } catch { /* this browser only */ }
      set({ enabled: on });
    },
    load: async () => {
      try { set({ view: await apiGet<WeatherView>('/api/senses/weather') }); } catch { /* keep the last */ }
    },
    setCity: (name) => put({ city: name }),
    setOff: (off) => put({ off }),
  };
});

/** The sky to draw: the override, else the reading. */
export const useSky = (): Sky | null =>
  useSkyStore(useShallow((s) => s.force ?? skyOfView(s.view)));
