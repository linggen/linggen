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

test('the calendar has no coming soon scan button', async ({ page, world, httpRequests }) => {
  await openSettings(page, world.url, httpRequests);
  await page.getByRole('button', { name: 'Memory' }).click();
  await page.getByRole('button', { name: 'Days', exact: true }).click();
  const days = page.getByTestId('memory-subview-days');
  await expect(days.getByRole('button', { name: 'Dream now' })).toBeVisible();
  const yesterday = new Date();
  yesterday.setDate(yesterday.getDate() - 1);
  const date = [yesterday.getFullYear(), String(yesterday.getMonth() + 1).padStart(2, '0'), String(yesterday.getDate()).padStart(2, '0')].join('-');
  const cell = days.getByRole('button', { name: date, exact: true });
  if (await cell.count()) await cell.click();
  await expect(page.locator('button[title="Coming with the next update."]')).toHaveCount(0);
});

test('the Memory settings show the store\'s two values and keep a change after a reload', async ({ page, world, httpRequests }) => {
  await page.addInitScript(watchAsked);
  await openSettings(page, world.url, httpRequests);
  await page.getByRole('button', { name: 'Memory' }).click();
  await page.getByRole('button', { name: 'Settings', exact: true }).last().click();

  const days = page.getByLabel('Keep short-term rows for (days)');
  const score = page.getByLabel('Recall threshold (0–1)');
  await expect(days).toHaveValue('7');
  await expect(score).toHaveValue('0.7');
  await expect(page.getByRole('heading', { name: 'Other machines' })).toBeVisible();

  const save = page.getByRole('button', { name: 'Save', exact: true });
  await expect(save).toBeDisabled();
  await days.fill('21');
  await expect(save).toBeEnabled();
  await save.click();
  await expect(page.getByText('Saved', { exact: true })).toBeVisible();
  await expect(save).toBeDisabled();
  expect(await asked(page)).toContain('PUT /api/ling-mem/config');

  // A fresh load of the page (the address bar now reads /settings, whose
  // direct load would be a plain-HTTP route), then back to the same place.
  await openSettings(page, world.url, httpRequests);
  await page.getByRole('button', { name: 'Memory' }).click();
  await page.getByRole('button', { name: 'Settings', exact: true }).last().click();
  await expect(page.getByLabel('Keep short-term rows for (days)')).toHaveValue('21');
  await expect(page.getByLabel('Recall threshold (0–1)')).toHaveValue('0.7');
  expect(await asked(page)).toContain('GET /api/ling-mem/config');
});
