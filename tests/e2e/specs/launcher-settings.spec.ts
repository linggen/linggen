import { test, expect } from '../support/world';

test('launcher settings survive reload and preserve the selected app when leaving', async ({ page, world }) => {
  await page.goto(`${world.url}/?launcher=1`);
  const app = page.locator('iframe[title="dice"]');
  const dice = app.contentFrame();
  const ready = dice.getByText('Harbor Table: Alex rolled 4');
  const back = page.getByRole('button', { name: 'Back', exact: true });
  await expect(ready).toBeVisible();
  await page.getByTitle('Settings', { exact: true }).click();
  await expect(page).toHaveURL(`${world.url}/settings`);
  await page.reload();
  await expect(back).toBeVisible();
  await expect.poll(() => page.evaluate(() => Reflect.get(window, '__LINGGEN_VIEW__'))).toBe('launcher');
  await expect(ready).toBeAttached();
  await expect(app).toBeHidden();
  const loadedAt = await ready.evaluate(() => performance.timeOrigin);

  await back.click();
  await expect(ready).toBeVisible();
  expect(await ready.evaluate(() => performance.timeOrigin)).toBe(loadedAt);
  await ready.evaluate((el) => {
    (el as HTMLElement).tabIndex = -1;
    (el as HTMLElement).focus();
  });
  // The native menu can open settings while input focus is inside the app.
  await page.evaluate(() => window.postMessage({ type: 'linggen:show-settings' }, location.origin));
  await expect(back).toBeVisible();
  await expect(app).toBeHidden();
  await expect(page.locator('[tabindex="-1"]').filter({ has: back })).toBeFocused();
  await page.keyboard.press('Escape');
  await expect(ready).toBeVisible();
  expect(await ready.evaluate(() => performance.timeOrigin)).toBe(loadedAt);

  const embedded = dice.frameLocator('iframe[title="Settings regression chat"]');
  await ready.evaluate(() => {
    const frame = document.createElement('iframe');
    frame.title = 'Settings regression chat';
    frame.src = '/embed';
    document.body.prepend(frame);
  });
  const input = embedded.locator('textarea').first();
  await expect(input).toBeVisible();
  expect(await input.evaluate(() => Reflect.get(window, '__LINGGEN_VIEW__'))).toBe('embed');
});

const shots = process.env.SETTINGS_SHOTS;

for (const face of ['web', 'launcher'] as const) {
  test(`${face} Settings → Apps lists only the declared page`, async ({ page, world }) => {
    if (face === 'launcher') {
      await page.goto(`${world.url}/?launcher=1`);
      await expect(page.frameLocator('iframe[title="dice"]').locator('body')).toBeVisible();
      await page.getByTitle('Settings', { exact: true }).click();
    } else {
      await page.goto(`${world.url}/settings`);
    }
    const apps = page.locator('aside > div').filter({ has: page.getByText('Apps', { exact: true }) });
    await expect(apps.getByRole('button')).toHaveText(['Fixture Options']);
    await apps.getByRole('button', { name: 'Fixture Options' }).click();
    const frame = page.locator('iframe[title="Fixture Options"]');
    await expect(frame).toHaveAttribute('src', '/apps/fixture-options/pages/options.html?embed=1');
    await expect(frame.contentFrame().getByRole('heading', { name: 'Fixture Options' })).toBeVisible();
    if (shots) await page.screenshot({ path: `${shots}/${face}.png` });
  });
}
