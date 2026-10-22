// node --test ui/tests/*.test.mts
// How a chat picture is drawn, and what a page may hand addMessage.
import { test } from 'node:test';
import assert from 'node:assert/strict';
import { imageSource, pageImages, pageSender } from '../src/lib/chatImage.mts';

test('a kept picture is fetched over the channel; a sent one is drawn inline', () => {
  assert.deepEqual(imageSource('/api/sessions/images?session_id=s&name=ab.png'), { kind: 'fetch', url: '/api/sessions/images?session_id=s&name=ab.png' });
  assert.deepEqual(imageSource('data:image/jpeg;base64,AAA'), { kind: 'inline', src: 'data:image/jpeg;base64,AAA' });
  assert.deepEqual(imageSource('iVBORw0'), { kind: 'inline', src: 'data:image/png;base64,iVBORw0' });
});

test("a page's turn takes an agent id as sender and only string pictures", () => {
  assert.equal(pageSender('yinyue'), 'yinyue');
  assert.equal(pageSender('<b>x</b>'), null);
  assert.equal(pageSender(undefined), null);
  assert.deepEqual(pageImages(['a', 3, '', 'b']), ['a', 'b']);
  assert.deepEqual(pageImages('a'), []);
  assert.equal(pageImages(Array(12).fill('x')).length, 8);
});

test("an answer's picture past the cache cap loads from its address", () => {
  assert.deepEqual(imageSource('https://img.example/k.jpg'), { kind: 'inline', src: 'https://img.example/k.jpg' });
});
