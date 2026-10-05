// Web UI tests on hermetic worlds (doc/test-design.md § 2).
//   ./scripts/check.sh e2e        or   cd tests/e2e && npx playwright test
// Each test gets its own world (tests/world: fixtures home in /var/tmp, env
// cleared, the debug `ling --web` on a random port, llmposter, a scratch
// ling-mem). Chromium only, with fake media devices so the UI's real WebRTC
// transport carries everything.
import { defineConfig, devices } from '@playwright/test';

export default defineConfig({
  testDir: './specs',
  globalSetup: './support/global-setup.ts',
  timeout: 90_000,
  expect: { timeout: 15_000 },
  fullyParallel: true,
  workers: process.env.E2E_WORKERS ? Number(process.env.E2E_WORKERS) : 3,
  retries: 0,
  reporter: [['list'], ['html', { open: 'never' }]],
  use: {
    ...devices['Desktop Chrome'],
    viewport: { width: 1400, height: 900 },
    screenshot: 'only-on-failure',
    trace: (process.env.E2E_TRACE as 'on' | undefined) ?? 'retain-on-failure',
    launchOptions: {
      args: ['--use-fake-ui-for-media-stream', '--use-fake-device-for-media-stream'],
    },
  },
  projects: [{ name: 'chromium' }],
});
