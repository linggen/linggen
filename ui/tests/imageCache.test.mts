// node --test ui/tests/*.test.mts (Node strips the types).
// A sent picture is kept for its own message only. Found by the whole-code
// review (2026-11-19): the cache was keyed by sender and words, so a picture
// sent with "look" in one session showed on another session's "look".
import { test } from 'node:test';
import assert from 'node:assert/strict';
import { cacheImages, clearImageCache, restoreImages } from '../src/lib/imageCache.mts';
import type { ChatMessage } from '../src/types.ts';

const row = (over: Partial<ChatMessage>): ChatMessage =>
  ({ role: 'user', from: 'user', to: 'ling', text: 'look', timestamp: '', ...over }) as ChatMessage;
const pic = ['data:image/png;base64,AA'];

test("a picture comes back on its own message's row", () => {
  clearImageCache();
  cacheImages('sess-a', row({ clientId: 'c-1', images: pic }));
  // The engine's row of the same message, drawn without the picture.
  assert.deepEqual(restoreImages('sess-a', row({ clientId: 'c-1' })).images, pic);
});

test("another session's same words get no picture", () => {
  clearImageCache();
  cacheImages('sess-a', row({ clientId: 'c-1', images: pic }));
  assert.equal(restoreImages('sess-b', row({ clientId: 'c-1' })).images, undefined);
  assert.equal(restoreImages('sess-b', row({})).images, undefined);
});

test('another message with the same words gets no picture', () => {
  clearImageCache();
  cacheImages('sess-a', row({ clientId: 'c-1', images: pic }));
  assert.equal(restoreImages('sess-a', row({ clientId: 'c-2' })).images, undefined);
  assert.equal(restoreImages('sess-a', row({})).images, undefined);
});
