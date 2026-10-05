// A write outside the session's grant asks; Deny writes nothing and the run
// goes on to its reply.
import { existsSync } from 'node:fs';
import path from 'node:path';
import { test, expect, expectWebRtc, expectScriptedCalls, chat, send, shown } from '../support/world';

const FERRY = 'sess-1790000002-c0ac7ed2';

test.use({
  scenario: {
    scripts: {
      ling: [{ tool: 'Write', args: { path: 'probe.txt', content: 'hello' } }, { text: 'Left it.' }],
    },
  },
});

test('a permission ask renders and Deny works', async ({ page, world, httpRequests }) => {
  await page.goto(`${world.url}/?session=${FERRY}`);
  await expectWebRtc(page, httpRequests);
  await send(page, 'Write probe.txt saying hello.');

  const c = chat(page);
  await expect(c.getByText('Permission required', { exact: false })).toBeVisible();
  await expect(c.getByText('Write probe.txt', { exact: true })).toBeVisible();
  await c.getByRole('button', { name: /^Deny/ }).click();

  // The denied call is shown as such, then the run's reply.
  await expect(c.getByText(/Permission denied by user: Write/)).toBeVisible();
  await expect(shown(page, 'Left it.')).toHaveCount(1);
  await expect(c.getByRole('button', { name: /^Deny/ })).toHaveCount(0);
  expect(existsSync(path.join(world.work, 'probe.txt'))).toBe(false);
  await expectScriptedCalls(world, ['ling', 'ling']);
});
