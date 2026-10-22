// A chat picture's drawable src: inline as it is, or fetched over the data
// channel when it is the engine's kept copy (lib/chatImage.mts).
import { useEffect, useState } from 'react';
import { imageSource } from '../../lib/chatImage.mts';

export function useImageSrc(image: string): string | null {
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
