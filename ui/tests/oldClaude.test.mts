// The lock for old Claude sessions (doc/code-style.md, principle 2): a
// session the Claude Agent SDK ran up to 2.5.1 is told apart in one place,
// `isOldClaude()` in lib/oldClaude.mts. It fails when the runtime's name is
// read anywhere else in the page's source (the generated wire types aside).
import assert from 'node:assert/strict';
import { readFileSync, readdirSync, statSync } from 'node:fs';
import { join, relative } from 'node:path';
import { test } from 'node:test';
import { fileURLToPath } from 'node:url';
import { OLD_CLAUDE_LINE, continuedLine, isOldClaude } from '../src/lib/oldClaude.mts';

const root = fileURLToPath(new URL('../src/', import.meta.url));

function files(dir: string): string[] {
  return readdirSync(dir).flatMap((name) => {
    const path = join(dir, name);
    return statSync(path).isDirectory() ? files(path) : [path];
  });
}

test('only lib/oldClaude.mts names the old Claude runtime', () => {
  const others = files(root)
    .map((path) => relative(root, path))
    .filter((path) => /\.(tsx?|mts)$/.test(path))
    .filter((path) => path !== 'lib/oldClaude.mts' && !path.startsWith('types/generated/'))
    .filter((path) => readFileSync(join(root, path), 'utf8').includes('claude_agent_sdk'));
  assert.deepEqual(others, []);
});

test('an old Claude session is one the SDK ran; any other is not', () => {
  assert.equal(isOldClaude({ runtime: 'claude_agent_sdk' }), true);
  assert.equal(isOldClaude({ runtime: 'linggen' }), false);
  assert.equal(isOldClaude({}), false);
  assert.equal(isOldClaude(null), false);
});

test('what the page says about an old Claude session is one short line', () => {
  const words = (line: string) => line.split(/\s+/).filter(Boolean).length;
  assert.ok(words(OLD_CLAUDE_LINE) <= 20, OLD_CLAUDE_LINE);
  assert.ok(words(continuedLine('claude-sonnet-4-5')) <= 20);
  assert.ok(continuedLine('claude-sonnet-4-5').includes('claude-sonnet-4-5'));
});
