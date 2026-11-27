// Settings → Models shows Linggen Cloud's models in the block under the default
// card: the ids the site's price table holds (the scenario's `cloudModels`, the
// kept answer), each one to tick. The page draws `GET /api/providers`, for both
// ways into the page — and a failed provider list says so and offers Retry.
import type { Page } from '@playwright/test';
import { test, expect, expectWebRtc } from '../support/world';

const CLOUD = { updated: '2026-11-26', starter: 'stand-in-flash', models: [{ id: 'stand-in-flash', label: 'Stand-in Flash' }, { id: 'stand-in-max', label: 'Stand-in Max' }] };
test.use({ scenario: { cloudModels: CLOUD } });

/** Open the Linggen Cloud row: every model the site lists is there to tick. */
async function expectCloudModels(page: Page) {
  const group = page.getByTestId('ready-to-use');
  await expect(group).toBeVisible();
  await group.getByTestId('provider-row').filter({ hasText: 'Linggen Cloud' }).getByRole('button', { name: /Linggen Cloud/ }).click();
  for (const m of CLOUD.models) await expect(group.getByTestId('cloud-model').filter({ hasText: m.label })).toBeVisible();
}

test('opening /settings/models directly shows every model the site lists for Linggen Cloud', async ({ page, world, httpRequests }) => {
  await page.goto(`${world.url}/settings/models`);
  await expectWebRtc(page, httpRequests);
  await expectCloudModels(page);
});

test('going to Models from the settings home shows every model the site lists for Linggen Cloud', async ({ page, world, httpRequests }) => {
  await page.goto(`${world.url}/`);
  await page.getByTitle('Settings', { exact: true }).click();
  await expectWebRtc(page, httpRequests);
  await page.getByRole('button', { name: 'Models', exact: true }).click();
  await expectCloudModels(page);
});

test('a provider list that cannot be loaded says so and offers a retry', async ({ page, world, httpRequests }) => {
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
  await expectCloudModels(page);
});
