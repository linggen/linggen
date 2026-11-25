// Pictures of the real Models page for the owner to look at: both themes, in
// Chinese, at 1280 and 420 px — the page, Anthropic open, the move notice, and
// 添加服务商. Not a check: it runs only when MODELS_SHOTS_DIR names a folder.
import { mkdirSync } from 'node:fs';
import path from 'node:path';
import { test, expect, expectWebRtc } from '../support/world';

const DIR = process.env.MODELS_SHOTS_DIR;
const LISTS = { anthropic: ['claude-opus-5', 'claude-opus-5-5', 'claude-sonnet-5-5', 'claude-haiku-5-5'] };
const PINNED_OPUS = `
[[models]]
id = "seed-opus"
provider = "anthropic"
url = "https://api.anthropic.com"
model = "claude-opus-5"
[models.family_binding]
family = "claude-opus"
preset = "anthropic"
mode = "pinned"
`;

test.skip(!DIR, 'set MODELS_SHOTS_DIR to take the pictures');
test.use({ scenario: { families: { today: '2026-11-23', lists: LISTS }, configAppend: PINNED_OPUS, defaultModel: 'seed-opus' } });

for (const [width, theme] of [[1280, 'light'], [420, 'light'], [1280, 'dark'], [420, 'dark']] as const) {
  test(`pictures at ${width} px in ${theme}`, async ({ page, world, httpRequests }) => {
    await page.emulateMedia({ colorScheme: theme });
    // Tall enough that the settings' own scroll box shows the whole page.
    await page.setViewportSize({ width, height: width < 600 ? 3200 : 2000 });
    mkdirSync(DIR!, { recursive: true });
    const shot = (name: string) => page.screenshot({ path: path.join(DIR!, `${name}-${theme}-${width}.png`) });
    await page.goto(`${world.url}/settings/models?lang=zh`);
    await expectWebRtc(page, httpRequests);
    await expect(page.getByTestId('default-card').getByText('灵境插图等', { exact: true })).toBeVisible();
    await shot('page');

    const row = page.getByTestId('provider-row').filter({ hasText: 'Anthropic' }).first();
    await row.getByRole('button').first().click();
    await row.getByTestId('key-input').fill('test-only-marker');
    await row.getByTestId('key-input').press('Enter');
    await expect(row.getByTestId('key-set')).toBeVisible();
    const opus = row.getByTestId('family-row').filter({ has: page.getByLabel('Opus', { exact: true }) });
    await opus.getByRole('button').first().click();
    await page.getByRole('menuitem').filter({ hasText: '恢复自动跟上' }).click();
    await expect(opus.getByTestId('move-notice')).toBeVisible();
    await shot('claude-notice');
    await opus.getByTestId('move-notice').getByRole('button', { name: '知道了' }).click();
    await expect(opus.getByTestId('move-notice')).toBeHidden();
    await shot('claude');

    const summary = page.getByTestId('chat-fallback-summary');
    await page.locator('[aria-controls="chat-fallback-editor"]').click();
    const add = page.getByRole('combobox', { name: '添加备用' });
    const sonnet = row.getByRole('checkbox', { name: 'Sonnet', exact: true });
    await sonnet.click();
    await expect(sonnet).toBeChecked();
    const sonnetOption = add.getByRole('option', { name: /^Claude Sonnet 5\.5 · / });
    await expect(sonnetOption).toHaveCount(1);
    await add.selectOption(await sonnetOption.getAttribute('value') ?? '');
    await expect(page.getByTestId('chat-fallback')).toHaveCount(1);
    await add.selectOption('fake-sol');
    await expect(page.getByTestId('chat-fallback')).toHaveCount(2);
    await expect(page.getByText('备用只在开启自动换模型时用', { exact: true })).toBeVisible();
    await shot('fallback-editor');
    await page.locator('[aria-controls="chat-fallback-editor"]').click();
    await expect(summary).toContainText('→');
    await shot('fallbacks');

    await expect(row.getByTestId('custom-models')).toBeVisible();
    await expect(row.getByTestId('provider-address')).toBeVisible();
    await shot('claude-open');

    await page.getByTestId('add-provider-button').click();
    await expect(page.getByTestId('add-provider')).toBeVisible();
    await shot('add-provider');
    await page.getByTestId('add-provider').getByRole('button', { name: '关闭' }).click();
    await page.locator('[data-testid="provider-row"][data-name="openai"]').getByRole('button').first().click();
    await shot('openai');
  });
}
