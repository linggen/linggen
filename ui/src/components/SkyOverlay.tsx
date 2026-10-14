/**
 * The live weather over the shell (/shared/sky.js, bundled, `overlay` look):
 * thin streaks, soft flakes, edge wisps of fog drawn ABOVE the UI, with no
 * fill or tint — clicks, selection, scrolling and typing pass straight
 * through. It falls evenly over the whole window, thin and faint enough
 * that the text under it reads as before. Clear and cloudy skies draw
 * nothing; storms bring no lightning; reduced motion draws nothing. Settings →
 * General → Weather → "Live sky" turns it off (this browser).
 */
import React, { useEffect, useRef } from 'react';
import { mountSky, type SkyHandle } from '@linggen/sky';
import { useSky, useSkyStore, useWeatherPoll } from '../stores/skyStore';

export const SkyOverlay: React.FC = () => {
  useWeatherPoll();
  const host = useRef<HTMLDivElement>(null);
  const handle = useRef<SkyHandle | null>(null);
  const sky = useSky();
  const enabled = useSkyStore((s) => s.enabled);
  const showing = enabled && !!sky;

  useEffect(() => {
    if (!showing || !host.current || !sky) return;
    const painter = mountSky(host.current, { ...sky, overlay: true });
    handle.current = painter;
    return () => { painter.destroy(); handle.current = null; };
    // Mounted once per showing; kind changes go through set() below.
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [showing]);
  useEffect(() => { if (sky) handle.current?.set(sky); }, [sky]);

  if (!showing) return null;
  return <div ref={host} className="fixed inset-0 z-[2147483000] pointer-events-none" aria-hidden />;
};
