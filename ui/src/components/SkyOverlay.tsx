/**
 * The live weather over the shell (/shared/sky.js, bundled, `overlay` look):
 * thin streaks, soft flakes, edge wisps of fog drawn ABOVE the UI, with no
 * fill or tint — clicks, selection, scrolling and typing pass straight
 * through. It falls evenly over the whole window, thin and faint enough
 * that the text under it reads as before. Clear and cloudy skies draw
 * nothing; storms bring no lightning; reduced motion keeps only a still
 * frame. The wind leans and carries it; the season adds a few faint things;
 * a festival its particles.
 *
 * Behind it, the day's picture (Hanli, 2026-10-15): the festival's painting
 * on its day, else the 节气's — a background at a few percent, in this same
 * click-through layer, never a card or a banner, never blurred or tinted;
 * it fades out toward the middle of the window, where the work is, so the
 * text's contrast is kept. Off with the overlay and with reduced motion.
 * Settings → General → Weather → "Live sky" turns it all off (this browser).
 */
import React, { useEffect, useRef, useSyncExternalStore } from 'react';
import { mountSky, type SkyHandle } from '@linggen/sky';
import { useSky, useSkyBackdrop, useSkyStore, useWeatherPoll } from '../stores/skyStore';

const CALM = '(prefers-reduced-motion: reduce)';
const calmNow = () => (typeof matchMedia === 'function' ? matchMedia(CALM).matches : false);
const onCalm = (fn: () => void) => {
  if (typeof matchMedia !== 'function') return () => {};
  const mq = matchMedia(CALM);
  mq.addEventListener('change', fn);
  return () => mq.removeEventListener('change', fn);
};

/* How faint the picture lies over the UI. */
const PICTURE_OPACITY = 0.07;

const Picture: React.FC<{ src: string }> = ({ src }) => (
  <div
    aria-hidden
    className="fixed inset-0 z-[2147482999] pointer-events-none bg-center bg-cover mix-blend-multiply dark:mix-blend-screen"
    style={{
      backgroundImage: `url("/${src}")`,
      opacity: PICTURE_OPACITY,
      maskImage: 'radial-gradient(ellipse 70% 65% at 50% 50%, transparent 35%, black 90%)',
      WebkitMaskImage: 'radial-gradient(ellipse 70% 65% at 50% 50%, transparent 35%, black 90%)',
    }}
  />
);

export const SkyOverlay: React.FC = () => {
  useWeatherPoll();
  const host = useRef<HTMLDivElement>(null);
  const handle = useRef<SkyHandle | null>(null);
  const sky = useSky();
  const picture = useSkyBackdrop();
  const calm = useSyncExternalStore(onCalm, calmNow, () => false);
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

  if (!enabled) return null;
  return (
    <>
      {picture && !calm ? <Picture src={picture} /> : null}
      {showing ? <div ref={host} className="fixed inset-0 z-[2147483000] pointer-events-none" aria-hidden /> : null}
    </>
  );
};
