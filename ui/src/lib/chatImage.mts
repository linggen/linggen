// How a chat picture is drawn. A message's `images` holds what the sending
// surface had (bare base64, or a data: URL) or, once restored from the
// session, the engine's URL for the kept copy (`/api/sessions/images?…`),
// which is fetched over the data channel — never an <img src> to /api,
// which would be plain HTTP.

export type ChatImageSource =
  | { kind: 'inline'; src: string }
  | { kind: 'fetch'; url: string };

export function imageSource(image: string): ChatImageSource {
  if (image.startsWith('data:') || image.startsWith('blob:')) return { kind: 'inline', src: image };
  if (image.startsWith('/api/')) return { kind: 'fetch', url: image };
  // Bare base64: the browser sniffs the real kind from the bytes.
  return { kind: 'inline', src: `data:image/png;base64,${image}` };
}

/** A sender id a page may label its own turn with (`addMessage` `from`). */
export function pageSender(from: unknown): string | null {
  return typeof from === 'string' && /^[a-z0-9_-]{1,32}$/.test(from) ? from : null;
}

/** The pictures a page hands `addMessage`: strings only, at most 8. */
export function pageImages(images: unknown): string[] {
  if (!Array.isArray(images)) return [];
  return images.filter((i): i is string => typeof i === 'string' && i.length > 0).slice(0, 8);
}
