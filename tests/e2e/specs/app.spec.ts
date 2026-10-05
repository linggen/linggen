// An app skill's web page (the dice fixture's index.html) loads at
// /apps/<name>/ — opened the way the UI opens it, from the Skills card.
import { test, expect, expectWebRtc } from '../support/world';

test("an app skill's page opens from the Skills card", async ({ page, world, httpRequests }) => {
  await page.goto(world.url);
  await expectWebRtc(page, httpRequests);
  const [app] = await Promise.all([
    page.waitForEvent('popup'),
    page.getByText('/dice', { exact: true }).click(),
  ]);
  await expect(app).toHaveURL(/\/apps\/dice\/index\.html$/);
  await expect(app.getByRole('heading', { name: 'Dice table' })).toBeVisible();
  // It read its own data from the skill folder.
  await expect(app.getByText('Harbor Table: Alex rolled 4')).toBeVisible();
});
