// Settings → Tools & MCP → Outside agents: the owner opens a private app to
// agents outside Linggen with one switch, and it holds — the page shows the
// same after a reload, and the MCP door's `app` tool answers for it.
import { mkdirSync, writeFileSync } from 'node:fs';
import type { APIRequestContext } from '@playwright/test';
import { test, expect, expectWebRtc } from '../support/world';

/** A made-up private app with one read action offered anywhere. */
async function installVault(request: APIRequestContext, url: string, home: string) {
  const dir = `${home}/skills/vault`;
  mkdirSync(dir, { recursive: true });
  writeFileSync(
    `${dir}/SKILL.md`,
    `---\nname: vault\ndescription: Keeps the money.\nallowed-tools: []\nprivate-talk: true\ncwd: ${dir}\npermission:\n  paths:\n    - { path: ${dir}, mode: edit }\ntools:\n  - name: Balance\n    description: What is in the vault.\n    cmd: "echo balance is 42"\n    tier: read\n    reach: anywhere\n---\n\nYou keep the vault.\n`,
  );
  await request.post(`${url}/api/skills/reload`, { data: {} });
}

/** The door's `app` tool, op call, as an outside agent at this Mac asks it. */
async function balance(request: APIRequestContext, url: string): Promise<{ text: string; isError: boolean }> {
  const reply = await request.post(`${url}/mcp`, {
    data: {
      jsonrpc: '2.0', id: 1, method: 'tools/call',
      params: { name: 'app', arguments: { op: 'call', app: 'vault', action: 'Balance' } },
    },
  });
  const result = (await reply.json()).result;
  return { text: result.content[0].text, isError: result.isError === true };
}

test('the owner opens a private app to outside agents, and it holds after a reload', async ({ page, world, request, httpRequests }) => {
  await installVault(request, world.url, world.linggenHome);
  expect((await balance(request, world.url)).isError, 'closed until opened').toBe(true);

  await page.goto(`${world.url}/settings`);
  await expectWebRtc(page, httpRequests);
  await page.getByRole('button', { name: 'Tools & MCP', exact: true }).click();
  const row = page.locator('[data-testid="outside-app"][data-app="vault"]');
  await expect(row).toBeVisible();
  const toggle = row.getByRole('switch');
  await expect(toggle).toHaveAttribute('aria-checked', 'false');

  await toggle.click();
  await expect(toggle).toHaveAttribute('aria-checked', 'true');
  const opened = await balance(request, world.url);
  expect(opened, 'the door answers for it').toEqual({ text: 'balance is 42\n', isError: false });

  await page.reload();
  await expectWebRtc(page, httpRequests);
  await page.getByRole('button', { name: 'Tools & MCP', exact: true }).click();
  await expect(row.getByRole('switch')).toHaveAttribute('aria-checked', 'true');

  await row.getByRole('switch').click();
  await expect(row.getByRole('switch')).toHaveAttribute('aria-checked', 'false');
  expect((await balance(request, world.url)).isError, 'closed again').toBe(true);
});
