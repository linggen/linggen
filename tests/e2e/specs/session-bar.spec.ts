import { test, expect, expectWebRtc, send } from '../support/world';

const SESSION = 'sess-1790000002-c0ac7ed2';

test.use({ viewport: { width: 700, height: 900 }, scenario: {
  scripts: { ling: [
    { tool: 'Bash', args: { cmd: 'sleep 30', run_in_background: true } },
    { text: 'Task started.' },
  ] },
} });

test('the narrow session bar keeps its task and menu controls in one row', async ({ page, world, httpRequests }) => {
  await page.goto(`${world.url}/?session=${SESSION}`);
  await expectWebRtc(page, httpRequests);
  const bar = page.getByTestId('session-bar');
  await expect(bar.getByTestId('subagent-pane-toggle')).toHaveCount(0);
  const initialHeight = await bar.evaluate((el) => el.getBoundingClientRect().height);
  await send(page, 'Start a background task');
  const permission = page.getByTestId('permission-directory');
  if (await permission.last().isVisible({ timeout: 3000 }).catch(() => false)) await permission.last().click();
  const chip = bar.getByTestId('subagent-pane-toggle');
  await expect(chip).toBeVisible();
  await expect(chip).toContainText('子任务 1');
  await expect(chip.locator('[aria-label="running"]')).toBeVisible();
  const dimensions = await bar.evaluate((el) => {
    const rect = el.getBoundingClientRect();
    return { height: rect.height, width: el.clientWidth, scrollWidth: el.scrollWidth };
  });
  expect(dimensions.height).toBeLessThanOrEqual(initialHeight + 1);
  expect(dimensions.scrollWidth).toBeLessThanOrEqual(dimensions.width);
  const menu = bar.getByRole('button', { name: 'More actions' });
  await expect(menu).toBeVisible();
  if (await chip.getAttribute('aria-expanded') === 'true') await chip.click();
  await expect(chip).toHaveAttribute('aria-expanded', 'false');
  await chip.click();
  await expect(chip).toHaveAttribute('aria-expanded', 'true');
  await expect(page.getByText('Subagents & tasks', { exact: false })).toBeVisible();
  await menu.click();
  await expect(bar.getByRole('menu')).toBeVisible();
});
