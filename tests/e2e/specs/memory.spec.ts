// Settings → Memory: the row browser over the engine's door. A fresh world
// holds no memory, so the tab shows its stats and an empty list; every call
// rode the data channel (the page fixture fails any plain-HTTP data request).
import type { Page } from '@playwright/test';
import { test, expect, expectWebRtc, watchAsked, asked } from '../support/world';

/** The page, then Settings through its own button (a direct visit to
 *  /settings would be a plain-HTTP page load of a route). */
async function openSettings(page: Page, url: string, httpRequests: string[]) {
  await page.goto(url);
  await expectWebRtc(page, httpRequests);
  await page.getByRole('button', { name: 'Settings', exact: true }).click();
  await expect(page.getByText('Back')).toBeVisible();
}

test('the Memory tab lists an empty store and asks only over the data channel', async ({ page, world, httpRequests }) => {
  await page.addInitScript(watchAsked);
  await openSettings(page, world.url, httpRequests);
  await page.getByRole('button', { name: 'Memory' }).click();

  await expect(page.getByRole('tab', { name: 'Long-term' })).toBeVisible();
  await expect(page.getByTestId('memory-stats')).toContainText('0 rows');
  await expect(page.getByTestId('memory-empty')).toHaveText('Memory is empty.');

  // A tier tab narrows the same list.
  await page.getByRole('tab', { name: 'Core' }).click();
  await expect(page.getByTestId('memory-empty')).toHaveText('No core rows yet.');

  const calls = await asked(page);
  expect(calls).toContain('POST /api/memory/stats');
  expect(calls).toContain('POST /api/memory/list');
});

test('the Memory entry sits in the Data group of the Settings sidebar', async ({ page, world, httpRequests }) => {
  await openSettings(page, world.url, httpRequests);
  await expect(page.locator('aside.md\\:flex')).toContainText('DataMemoryStorage');
  await page.getByRole('button', { name: 'Memory' }).click();
  await expect(page.getByTestId('memory-subview-rows')).toBeVisible();
});

test('the Memory tab shows review and days', async ({ page, world, httpRequests }) => {
  await openSettings(page, world.url, httpRequests);
  await page.getByRole('button', { name: 'Memory' }).click();
  await page.getByRole('button', { name: 'Review', exact: true }).click();
  await expect(page.getByText('Nothing waits for review.')).toBeVisible();
  await page.getByRole('button', { name: 'Days', exact: true }).click();
  await expect(page.getByTestId('memory-day').first().or(page.getByText('No days yet.'))).toBeVisible();
});
