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
 * BEHIND the UI, the day's picture (Hanli, 2026-11-09): the festival's
 * painting on its day, else the 节气's — a backdrop under the panels, never
 * over text, never a card or a banner, never blurred or tinted. It paints
 * the shell's base colour with the painting on it (ink-on-paper multiply in
 * light; plain and fainter in dark, so the paper doesn't glow), fading out
 * under the header. While it shows, <html data-sky-backdrop> turns the shell
 * surfaces translucent (--surface-* in index.css) so the trees and hills at
 * the edges read through; cards, inputs, code and menus stay opaque.
 * Off with the overlay and with reduced motion.
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

/* The painting's strength over the shell base, per mode. */
const PICTURE_LOOK = 'opacity-[.2] mix-blend-multiply dark:opacity-[.12] dark:mix-blend-normal';
/* Clear of the header: transparent to 56px, full by 140px. */
const HEADER_FADE = 'linear-gradient(to bottom, transparent 0, transparent 56px, black 140px)';

/** The day's painting behind the shell: base colour + the picture on it. */
const Backdrop: React.FC<{ src: string }> = ({ src }) => (
  <div aria-hidden className="fixed inset-0 -z-10 pointer-events-none bg-slate-100/70 dark:bg-[#0a0a0a]">
    <div
      className={`absolute inset-0 bg-center bg-cover ${PICTURE_LOOK}`}
      style={{ backgroundImage: `url("/${src}")`, maskImage: HEADER_FADE, WebkitMaskImage: HEADER_FADE }}
    />
  </div>
);

/** Mark <html> while the backdrop shows, so the shell surfaces go translucent. */
function useBackdropMark(on: boolean) {
  useEffect(() => {
    if (!on) return;
    const root = document.documentElement;
    root.dataset.skyBackdrop = '';
    return () => { delete root.dataset.skyBackdrop; };
  }, [on]);
}

export const SkyOverlay: React.FC = () => {
  useWeatherPoll();
  const host = useRef<HTMLDivElement>(null);
  const handle = useRef<SkyHandle | null>(null);
  const sky = useSky();
  const picture = useSkyBackdrop();
  const calm = useSyncExternalStore(onCalm, calmNow, () => false);
  const enabled = useSkyStore((s) => s.enabled);
  const showing = enabled && !!sky;
  const backdrop = enabled && !calm ? picture : null;
  useBackdropMark(!!backdrop);

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
      {backdrop ? <Backdrop src={backdrop} /> : null}
      {showing ? <div ref={host} className="fixed inset-0 z-[2147483000] pointer-events-none" aria-hidden /> : null}
    </>
  );
};
