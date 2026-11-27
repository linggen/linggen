// Settings → Models shows Linggen Cloud's built-in models in the block under the
// default model (the ChatGPT sign-in is a provider added below it). `GET /api/models` answers by who asks; over the
// channel the owner's own page was taken for no one, the list came back empty
// and the group vanished while the chat's picker (page_state) still listed
// them. The page now draws `GET /api/providers`: the test asks the engine for
// the built-in list the way the owner's machine does and holds the page to it,
// for both ways into the page — and holds a failed provider list to its
// failure line and Retry.
import type { APIRequestContext, Page } from '@playwright/test';
import { test, expect, expectWebRtc } from '../support/world';

/** The ids of the built-in rows the engine answers on loopback. */
async function builtinIds(world: { url: string }, request: APIRequestContext): Promise<string[]> {
  const rows: { id: string; is_builtin?: boolean }[] = await (await request.get(`${world.url}/api/models`)).json();
  // The ChatGPT built-ins sit on the ChatGPT provider's row, not in the top block.
  return rows.filter((m) => m.is_builtin && m.provider !== 'chatgpt').map((m) => m.id);
}

/** Every built-in id is on the page, in the Linggen Cloud block. */
async function expectBuiltinGroup(page: Page, ids: string[]) {
  const group = page.getByTestId('ready-to-use');
  await expect(group).toBeVisible();
  for (const id of ids) await expect(group.getByText(id).first()).toBeVisible();
}

test('the engine answers built-in models for the world', async ({ world, request }) => {
  expect((await builtinIds(world, request)).length, 'no built-in model in the engine answer: the lock below would hold nothing').toBeGreaterThan(0);
});

test('opening /settings/models directly shows every built-in model the engine lists', async ({ page, world, request, httpRequests }) => {
  const ids = await builtinIds(world, request);
  await page.goto(`${world.url}/settings/models`);
  await expectWebRtc(page, httpRequests);
  await expectBuiltinGroup(page, ids);
});

test('going to Models from the settings home shows every built-in model the engine lists', async ({ page, world, request, httpRequests }) => {
  const ids = await builtinIds(world, request);
  await page.goto(`${world.url}/`);
  await page.getByTitle('Settings', { exact: true }).click();
  await expectWebRtc(page, httpRequests);
  await page.getByRole('button', { name: 'Models', exact: true }).click();
  await expectBuiltinGroup(page, ids);
});

test('a provider list that cannot be loaded says so and offers a retry', async ({ page, world, request, httpRequests }) => {
  const ids = await builtinIds(world, request);
  await page.goto(`${world.url}/settings`);
  await expectWebRtc(page, httpRequests);
  await page.evaluate(() => {
    // The next read of /api/providers, through the page's own fetch, is refused.
    const ask = window.fetch.bind(window);
    let refused = false;
    window.fetch = ((input: RequestInfo | URL, init?: RequestInit) => {
      const url = typeof input === 'string' ? input : input instanceof URL ? input.pathname : input.url;
      if (!refused && url.split('?')[0].endsWith('/api/providers')) {
        refused = true;
        return Promise.resolve(new Response('no', { status: 500 }));
      }
      return ask(input, init);
    }) as typeof window.fetch;
  });
  await page.getByRole('button', { name: 'Models', exact: true }).click();
  await expect(page.getByTestId('providers-load-failed')).toBeVisible();
  await page.getByTestId('providers-load-failed').getByRole('button', { name: 'Retry' }).click();
  await expect(page.getByTestId('providers-load-failed')).toBeHidden();
  await expectBuiltinGroup(page, ids);
});
