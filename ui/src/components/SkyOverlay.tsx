/**
 * The live weather over the shell (/shared/sky.js, bundled, `overlay` look):
 * thin streaks, soft flakes, edge wisps of fog drawn ABOVE the UI, with no
 * fill or tint — clicks, selection, scrolling and typing pass straight
 * through. Over the chat column (`data-sky-cover`) only a sparser, fainter
 * layer falls, so the text there reads as before. Clear and cloudy skies draw
 * nothing; storms bring no lightning; reduced motion draws nothing. Settings →
 * General → Weather → "Live sky" turns it off (this browser).
 */
import React, { useEffect, useRef } from 'react';
import { mountSky, type SkyHandle } from '@linggen/sky';
import { useSky, useSkyStore, useWeatherPoll } from '../stores/skyStore';

const coverRects = () =>
  Array.from(document.querySelectorAll('[data-sky-cover]')).map((el) => el.getBoundingClientRect());

export const SkyOverlay: React.FC = () => {
  useWeatherPoll();
  const host = useRef<HTMLDivElement>(null);
  const handle = useRef<SkyHandle | null>(null);
  const sky = useSky();
  const enabled = useSkyStore((s) => s.enabled);
  const showing = enabled && !!sky;

  useEffect(() => {
    if (!showing || !host.current || !sky) return;
    const painter = mountSky(host.current, { ...sky, overlay: true, cover: coverRects });
    handle.current = painter;
    return () => { painter.destroy(); handle.current = null; };
    // Mounted once per showing; kind changes go through set() below.
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [showing]);
  useEffect(() => { if (sky) handle.current?.set(sky); }, [sky]);

  if (!showing) return null;
  return <div ref={host} className="fixed inset-0 z-[2147483000] pointer-events-none" aria-hidden />;
};
