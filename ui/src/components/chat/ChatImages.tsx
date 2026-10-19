// A turn's pictures: small thumbnails on the bubble; a click opens one large
// over the page (Esc or a click closes it). Kept pictures restored from the
// session are fetched over the data channel (lib/chatImage.mts).
import React, { useEffect, useState } from 'react';
import { createPortal } from 'react-dom';
import { imageSource } from '../../lib/chatImage.mts';

function useImageSrc(image: string): string | null {
  const source = imageSource(image);
  const [fetched, setFetched] = useState<string | null>(null);
  const url = source.kind === 'fetch' ? source.url : null;
  useEffect(() => {
    if (!url) return;
    let objectUrl: string | null = null;
    let live = true;
    fetch(url)
      .then((res) => (res.ok ? res.blob() : null))
      .then((blob) => {
        if (!blob || !live) return;
        objectUrl = URL.createObjectURL(blob);
        setFetched(objectUrl);
      })
      .catch(() => {});
    return () => {
      live = false;
      if (objectUrl) URL.revokeObjectURL(objectUrl);
    };
  }, [url]);
  return source.kind === 'inline' ? source.src : fetched;
}

function Lightbox({ src, onClose }: { src: string; onClose: () => void }) {
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
      <img src={src} alt="" className="max-w-full max-h-full object-contain rounded shadow-2xl" />
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
