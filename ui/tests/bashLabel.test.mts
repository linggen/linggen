// node --test ui/tests/*.test.mts (Node strips the types).
// A Bash call reads as what it does: its description, else its command
// made short — the tool row and the activity log alike.
import { test } from 'node:test';
import assert from 'node:assert/strict';
import { bashCommand, bashLabel, shortCommand, shortPath, BASH_LABEL_MAX } from '../src/lib/bashLabel.mts';

const RESTART =
  'cd /Users/someone/workspace/linggen/linggen && export PATH="$HOME/.cargo/bin:/opt/homebrew/bin:$HOME/.linggen/bin:$PATH" && scripts/restart-engine.sh 2>&1 | tail -3';

test('the description is the label when the call has one', () => {
  assert.equal(
    bashLabel({ command: RESTART, description: 'Build engine with the new pane and restart' }),
    'Build engine with the new pane and restart',
  );
  assert.equal(bashLabel({ cmd: 'ls', description: '  ' }), 'ls');
  assert.equal(bashLabel({ cmd: 'ls', description: 'a'.repeat(200) }).length, BASH_LABEL_MAX);
});

test('without one, the command made short', () => {
  assert.equal(bashLabel({ command: RESTART }), 'scripts/restart-engine.sh');
  assert.equal(bashLabel({ cmd: 'git status' }), 'git status');
  assert.equal(bashLabel({}), '');
  assert.equal(bashLabel(null), '');
});

test('set-up in front is dropped: cd, export, env prefixes', () => {
  assert.equal(shortCommand('cd ui && npm run build'), 'npm run build');
  assert.equal(shortCommand('export PATH="$HOME/.cargo/bin:$PATH"; cargo test --quiet'), 'cargo test --quiet');
  assert.equal(shortCommand('export A=1 B=\'x y\' && make'), 'make');
  assert.equal(shortCommand('RUST_LOG=debug FOO=1 cargo run -- --web'), 'cargo run -- --web');
});

test('output plumbing behind is dropped: 2>&1, | tail -N, | head -N', () => {
  assert.equal(shortCommand('cargo build 2>&1 | tail -n 20'), 'cargo build');
  assert.equal(shortCommand('git log --oneline | head -5'), 'git log --oneline');
  assert.equal(shortCommand('git status | head'), 'git status');
  assert.equal(shortCommand('make 2>&1'), 'make');
  // A pipe that does work stays.
  assert.equal(shortCommand('ps aux | grep ling'), 'ps aux | grep ling');
});

test('a command that is all set-up stays whole', () => {
  assert.equal(shortCommand('cd /tmp'), 'cd /tmp');
  assert.equal(shortCommand('FOO=1'), 'FOO=1');
});

test('paths: home is ~, deep ones their last part', () => {
  assert.equal(shortPath('/Users/someone/notes/todo.md'), '~/notes/todo.md');
  assert.equal(shortPath('/home/someone/a/b/c.txt'), 'c.txt');
  assert.equal(shortPath('$HOME/.linggen'), '~/.linggen');
  assert.equal(shortPath('/opt/homebrew/bin/node'), 'node');
  assert.equal(shortPath('/dev/null'), '/dev/null');
  assert.equal(
    shortCommand('/opt/homebrew/bin/node node_modules/vite/bin/vite.js build'),
    'node node_modules/vite/bin/vite.js build',
  );
  assert.equal(shortCommand('grep -rn foo /Users/someone/work/repo/src > /dev/null'), 'grep -rn foo src > /dev/null');
});

test('long commands are capped; lines are joined', () => {
  const long = shortCommand('echo ' + 'x'.repeat(300));
  assert.equal(long.length, BASH_LABEL_MAX);
  assert.ok(long.endsWith('…'));
  assert.equal(shortCommand('set -e\n  make\n'), 'set -e make');
  assert.equal(shortCommand('echo hi', 5), 'echo…');
});

test('the whole command stays for the expanded row', () => {
  assert.equal(bashCommand({ command: RESTART, description: 'Restart' }), RESTART);
  assert.equal(bashCommand({ cmd: ' ls ' }), 'ls');
});
