/**
 * The live weather as the shell's background (/shared/sky.js, bundled). The
 * canvas covers the viewport behind everything; every reading surface marked
 * `sky-panel` frosts over it, and `data-sky-cover` panels get only the slow,
 * sparse back layer — full weather shows in the gutters and the header.
 * Sets `html[data-sky]` to `glass` or `solid` (index.css § Sky).
 */
import React, { useEffect, useRef, useState } from 'react';
import { canFrost, mountSky, type SkyHandle } from '@linggen/sky';
import { skyMode } from '../lib/skyChoice.mts';
import { useSky, useSkyStore } from '../stores/skyStore';

const POLL_MS = 10 * 60 * 1000;

const coverRects = () =>
  Array.from(document.querySelectorAll('[data-sky-cover]')).map((el) => el.getBoundingClientRect());

/** Keep the weather view current while the page is visible. */
function useWeatherPoll() {
  const load = useSkyStore((s) => s.load);
  useEffect(() => {
    load();
    const timer = setInterval(() => { if (!document.hidden) load(); }, POLL_MS);
    return () => clearInterval(timer);
  }, [load]);
}

/** Mount the painter in `host` while there is a sky; follow its kind. */
function usePainter(host: React.RefObject<HTMLDivElement | null>, opts: { cover?: boolean; onStill?: (still: boolean) => void }) {
  const sky = useSky();
  const enabled = useSkyStore((s) => s.enabled);
  const handle = useRef<SkyHandle | null>(null);
  const showing = enabled && !!sky;
  useEffect(() => {
    if (!showing || !host.current || !sky) return;
    const painter = mountSky(host.current, {
      ...sky,
      cover: opts.cover ? coverRects : undefined,
      onMode: (m) => opts.onStill?.(m === 'still'),
    });
    handle.current = painter;
    return () => { painter.destroy(); handle.current = null; };
    // Mounted once per showing; kind changes go through set() below.
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [showing]);
  useEffect(() => { if (sky) handle.current?.set(sky); }, [sky]);
  return { sky, showing };
}

export const SkyBackground: React.FC = () => {
  useWeatherPoll();
  const host = useRef<HTMLDivElement>(null);
  const [still, setStill] = useState(false);
  const enabled = useSkyStore((s) => s.enabled);
  const { sky, showing } = usePainter(host, { cover: true, onStill: setStill });
  const mode = skyMode({ enabled, sky, frost: canFrost(), still });

  useEffect(() => {
    const root = document.documentElement;
    if (mode) root.dataset.sky = mode;
    else delete root.dataset.sky;
    return () => { delete root.dataset.sky; };
  }, [mode]);

  if (!showing) return null;
  return <div ref={host} className="fixed inset-0 -z-10 pointer-events-none" aria-hidden />;
};

const KIND_LABEL: Record<string, string> = {
  clear: 'Clear', cloudy: 'Cloudy', fog: 'Fog', rain: 'Rain', snow: 'Snow', storm: 'Storm',
};

/** An empty chat's window onto the sky: full weather, the caption below it. */
export const SkyWindow: React.FC = () => {
  const host = useRef<HTMLDivElement>(null);
  const { sky, showing } = usePainter(host, {});
  const reading = useSkyStore((s) => s.view?.reading);
  if (!showing || !sky) return null;
  const caption = [KIND_LABEL[sky.kind], reading && `${Math.round(reading.temp_c)}°C`, reading?.city].filter(Boolean).join(' · ');
  return (
    <div className="w-full max-w-md mx-auto mb-4">
      <div ref={host} className="relative h-28 rounded-xl overflow-hidden border border-slate-200 dark:border-white/10" aria-hidden />
      <div className="mt-1.5 text-[11px] text-slate-500 dark:text-slate-400">{caption}</div>
    </div>
  );
};
