/**
 * Client-side image cache for chat messages.
 *
 * The engine keeps a person's pictures with the row (restored as URLs), but
 * the sending surface's own base64 copy is cached here, keyed by
 * "from::normalizedText", so a bubble keeps its picture through merges and
 * reloads before the kept copy is fetched.
 */
import type { ChatMessage } from '../types';
import { normalizeMessageTextForDedup } from './messageUtils';

const _cache = new Map<string, string[]>();

function cacheKey(from: string, text: string): string {
  return `${from}::${normalizeMessageTextForDedup(text)}`;
}

/** Store images from a message into the cache. */
export function cacheImages(msg: ChatMessage): void {
  if (msg.images && msg.images.length > 0) {
    _cache.set(cacheKey(msg.from || msg.role || '', msg.text), msg.images);
  }
}

/** Restore images from the cache onto a message if it's missing them. */
export function restoreImages(msg: ChatMessage): ChatMessage {
  if (msg.images && msg.images.length > 0) return msg;
  const cached = _cache.get(cacheKey(msg.from || msg.role || '', msg.text));
  return cached ? { ...msg, images: cached } : msg;
}

/** Clear the entire image cache. */
export function clearImageCache(): void {
  _cache.clear();
}
