// A turn's pictures: small thumbnails on the bubble; a click opens one large
// over the page (Esc or a click closes it). Kept pictures restored from the
// session are fetched over the data channel (lib/chatImage.mts).
import React, { useEffect, useState } from 'react';
import { createPortal } from 'react-dom';
import { useImageSrc } from './useImageSrc';

/** One picture large over the page. `href`: the source it came from, a
 *  click-through under it. */
export function Lightbox({ src, onClose, caption, href }: { src: string; onClose: () => void; caption?: string; href?: string }) {
  useEffect(() => {
    const onKey = (e: KeyboardEvent) => { if (e.key === 'Escape') onClose(); };
    window.addEventListener('keydown', onKey);
    return () => window.removeEventListener('keydown', onKey);
  }, [onClose]);
  return createPortal(
    <div
      className="fixed inset-0 z-[1000] flex items-center justify-center bg-black/80 p-4 cursor-zoom-out"
      onClick={onClose}
      role="dialog"
      aria-label="Picture"
    >
      <div className="flex flex-col items-center gap-2 max-w-full max-h-full">
        <img src={src} alt={caption || ''} className="max-w-full max-h-[85vh] object-contain rounded shadow-2xl" />
        {(caption || href) && (
          <div className="text-sm text-white/90 text-center" onClick={(e) => e.stopPropagation()}>
            {caption}
            {href && (
              <a href={href} target="_blank" rel="noopener noreferrer" className="ml-2 underline text-white">
                Open source ↗
              </a>
            )}
          </div>
        )}
      </div>
    </div>,
    document.body,
  );
}

function Thumb({ image }: { image: string }) {
  const src = useImageSrc(image);
  const [open, setOpen] = useState(false);
  if (!src) {
    return <span className="inline-block w-16 h-16 rounded bg-slate-200 dark:bg-white/10 animate-pulse" />;
  }
  return (
    <>
      <button type="button" onClick={() => setOpen(true)} className="p-0 border-0 bg-transparent cursor-zoom-in" title="Open">
        <img src={src} alt="" className="h-16 max-w-[8rem] object-cover rounded border border-slate-200 dark:border-white/10" />
      </button>
      {open && <Lightbox src={src} onClose={() => setOpen(false)} />}
    </>
  );
}

export const ChatImages = React.memo(({ images }: { images?: string[] }) => {
  if (!images || images.length === 0) return null;
  return (
    <div className="flex flex-wrap gap-1.5 mt-1" data-no-i18n>
      {images.map((image, i) => <Thumb key={`${i}-${image.slice(-24)}`} image={image} />)}
    </div>
  );
});
