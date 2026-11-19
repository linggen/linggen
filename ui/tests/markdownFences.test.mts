// node --test ui/tests/*.test.mts (Node strips the types).
// The chat's markdown tidy-up never touches what is inside a code fence.
// Found by the whole-code review (2026-11-19): an indented `- ` line or a
// `1. ` line in a code block was moved or unindented, so the code shown and
// copied was not the code sent.
import { test } from 'node:test';
import assert from 'node:assert/strict';
import { normalizeMarkdownish } from '../src/components/chat/utils/markdown.mts';

const code = [
  '```yaml',
  'steps:',
  '  - run: make',
  '  - run: test',
  'note: 1. not a list',
  '<!-- kept -->',
  '```',
].join('\n');

test('a fenced block comes out as it went in', () => {
  const out = normalizeMarkdownish(`Here it is:\n\n${code}\n\nDone.`);
  assert.ok(out.includes(code), out);
});

test('a fence still open (the reply is streaming) is code to the end', () => {
  const open = '```sh\nls\n  - not a list\n1. nor this';
  assert.equal(normalizeMarkdownish(`Run:\n\n${open}`), `Run:\n\n${open}`);
});

test('a tilde fence is a fence too', () => {
  const tilde = '~~~\n  - keep\n~~~';
  assert.ok(normalizeMarkdownish(`x\n\n${tilde}`).includes(tilde));
});

test('prose outside the fence is still tidied', () => {
  assert.equal(normalizeMarkdownish('Steps: 1. one 2. two'), 'Steps:\n1. one\n2. two');
  assert.equal(normalizeMarkdownish('a <!-- note --> b\n\n```\n<!-- x -->\n```'), 'a  b\n\n```\n<!-- x -->\n```');
});
