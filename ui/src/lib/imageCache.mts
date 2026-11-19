/**
 * Client-side image cache for chat messages.
 *
 * The engine keeps a person's pictures with the row (restored as URLs), but
 * the sending surface's own base64 copy is cached here so a bubble keeps its
 * picture through merges and reloads before the kept copy is fetched. Keyed
 * by session and message (the bubble's `clientId`, which the engine keeps on
 * the saved row): never by the words, which another message — in this
 * session or another — may share. Pure, so node tests can hold it.
 */
import type { ChatMessage } from '../types.ts';

const _cache = new Map<string, string[]>();

function cacheKey(session: string | null | undefined, msg: ChatMessage): string | null {
  return msg.clientId ? `${session || ''}::${msg.clientId}` : null;
}

/** Store images from a message of `session` into the cache. */
export function cacheImages(session: string | null | undefined, msg: ChatMessage): void {
  const key = cacheKey(session, msg);
  if (key && msg.images && msg.images.length > 0) _cache.set(key, msg.images);
}

/** Restore images from the cache onto a message of `session` if it's missing them. */
export function restoreImages(session: string | null | undefined, msg: ChatMessage): ChatMessage {
  if (msg.images && msg.images.length > 0) return msg;
  const key = cacheKey(session, msg);
  const cached = key ? _cache.get(key) : undefined;
  return cached ? { ...msg, images: cached } : msg;
}

/** Clear the entire image cache. */
export function clearImageCache(): void {
  _cache.clear();
}
