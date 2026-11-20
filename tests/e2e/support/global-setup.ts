// Build what the worlds run, once per suite:
//   - ui/dist (`npm run build`) — the debug `ling` serves it from disk
//     (rust-embed reads the folder at run time in debug builds), so no
//     release build is needed and a rebuilt UI is picked up as is;
//   - the world binary (tests/world), which builds the debug `ling` it runs.
// E2E_SKIP_UI_BUILD=1 skips the UI build (it is already current).
import { execFileSync } from 'node:child_process';
import path from 'node:path';

const REPO = path.resolve(__dirname, '../../..');

function uiBuild() {
  if (process.env.E2E_SKIP_UI_BUILD) return;
  execFileSync('npm', ['run', 'build'], { cwd: path.join(REPO, 'ui'), stdio: 'inherit' });
}

/** Build the world in the same scratch environment as the Rust tests. */
function worldBuild(): string {
  const out = execFileSync(
    path.join(REPO, 'scripts/scratch-test.sh'),
    ['--test', 'world', '--no-run', '--message-format=json'],
    { cwd: REPO, encoding: 'utf8', stdio: ['ignore', 'pipe', 'inherit'], maxBuffer: 256 << 20 },
  );
  for (const line of out.split('\n')) {
    if (!line.startsWith('{')) continue;
    const msg = JSON.parse(line);
    if (msg.reason === 'compiler-artifact' && msg.target?.name === 'world' && msg.executable) {
      return msg.executable;
    }
  }
  throw new Error('cargo built no world binary');
}

export default function globalSetup() {
  // Inherited by the builds and Playwright workers. The Rust world also
  // sets its own opt-out because it clears the engine's inherited env.
  process.env.LINGGEN_NO_TELEMETRY = '1';
  uiBuild();
  process.env.LINGGEN_WORLD_BIN = worldBuild();
}
