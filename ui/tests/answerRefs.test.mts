// node --test ui/tests/*.test.mts
// An answer's numbers: chips inline, pictures after their paragraph, nothing
// for a number still being written.
import { test } from 'node:test';
import assert from 'node:assert/strict';
import { fromMarkdown } from 'mdast-util-from-markdown';
import { hidePartialRef, pictureCaption, remarkAnswerRefs, sourceLabel, videoByline } from '../src/lib/answerRefs.mts';

type Node = { type: string; value?: string; children?: Node[]; data?: { hName?: string; hProperties?: Record<string, string> } };
const tree = (md: string): Node => {
  const t = fromMarkdown(md) as unknown as Node;
  remarkAnswerRefs()(t as never);
  return t;
};
const shape = (n: Node): unknown =>
  n.type === 'text' ? n.value : n.data?.hName ? `${n.data.hName}:${Object.values(n.data.hProperties ?? {}).join()}` : { [n.type]: (n.children ?? []).map(shape) };

test("a paragraph's pictures follow it as one grid; every number stays in place inline", () => {
  const t = tree('Kettle A [turn1image2][turn1image1] from [turn1search1].\n\nNext.');
  assert.deepEqual(t.children!.map(shape), [
    { paragraph: ['Kettle A ', 'lg-ref:turn1image2', 'lg-ref:turn1image1', ' from ', 'lg-ref:turn1search1', '.'] },
    'lg-pictures:turn1image2 turn1image1',
    { paragraph: ['Next.'] },
  ]);
});

test("a video follows its paragraph in the same grid as pictures", () => {
  const t = tree('Watch how [turn4video1], or see [turn3image1].');
  assert.deepEqual(t.children!.map(shape), [
    { paragraph: ['Watch how ', 'lg-ref:turn4video1', ', or see ', 'lg-ref:turn3image1', '.'] },
    'lg-pictures:turn4video1 turn3image1',
  ]);
  assert.equal(hidePartialRef('Watch [turn4vid'), 'Watch ');
  assert.equal(videoByline({ kind: 'video', url: '', channel: 'Bridge Coffee', duration: '2:17' }), 'Bridge Coffee · 2:17');
  assert.equal(videoByline({ kind: 'video', url: '' }), '');
});

test('a list item keeps its pictures under its own words', () => {
  const t = tree('1. Fill it [turn3image1]\n2. Boil');
  const items = (t.children![0].children ?? []).map(shape);
  assert.deepEqual(items[0], { listItem: [{ paragraph: ['Fill it ', 'lg-ref:turn3image1'] }, 'lg-pictures:turn3image1'] });
  assert.deepEqual(items[1], { listItem: [{ paragraph: ['Boil'] }] });
});

test('a paragraph of nothing but pictures is the grid alone', () => {
  const t = tree('Look:\n\n[turn1image1] [turn1image2]\n\nDone.');
  assert.deepEqual(t.children!.map(shape), [
    { paragraph: ['Look:'] },
    'lg-pictures:turn1image1 turn1image2',
    { paragraph: ['Done.'] },
  ]);
});

test('other brackets and code are left alone', () => {
  const t = tree('See [notes] and `[turn1image1]` and [turn1news1].');
  assert.deepEqual(t.children!.map(shape), [
    { paragraph: ['See [notes] and ', { inlineCode: [] }, ' and [turn1news1].'] },
  ]);
});

test('a number still streaming is hidden', () => {
  assert.equal(hidePartialRef('Look [turn2ima'), 'Look ');
  assert.equal(hidePartialRef('Look ['), 'Look ');
  assert.equal(hidePartialRef('Look [turn2image1]'), 'Look [turn2image1]');
});

test('labels: the site for a source, a product before a caption', () => {
  assert.equal(sourceLabel({ kind: 'source', url: 'https://www.rtings.com/x' }), 'rtings.com');
  assert.equal(pictureCaption({ kind: 'picture', url: '', caption: 'kettle', name: 'Stagg', price: '165 USD' }), 'Stagg · 165 USD');
  assert.equal(pictureCaption({ kind: 'picture', url: '', caption: 'Step 1' }), 'Step 1');
});
