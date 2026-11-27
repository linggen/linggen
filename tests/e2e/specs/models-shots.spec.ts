// Pictures of the real Models page for the owner to look at: both themes, in
// Chinese, at 1280 and 420 px — the page, Anthropic open with a model added,
// and 添加服务商. Not a check: it runs only when MODELS_SHOTS_DIR names a folder.
import type { Page } from '@playwright/test';
import { mkdirSync } from 'node:fs';
import path from 'node:path';
import { test, expect, expectWebRtc } from '../support/world';

/** The own-providers section is folded until opened (or a provider has a problem). */
async function openOwnProviders(page: Page) {
  const toggle = page.getByTestId('own-providers-toggle');
  if ((await toggle.getAttribute('aria-expanded')) !== 'true') await toggle.click();
}

const DIR = process.env.MODELS_SHOTS_DIR;
const CLOUD = { updated: '2026-11-26', starter: 'stand-in-flash', models: [{ id: 'stand-in-flash', label: 'Stand-in Flash' }, { id: 'stand-in-max', label: 'Stand-in Max' }] };

test.skip(!DIR, 'set MODELS_SHOTS_DIR to take the pictures');
test.use({ scenario: { cloudModels: CLOUD } });

for (const [width, theme] of [[1280, 'light'], [420, 'light'], [1280, 'dark'], [420, 'dark']] as const) {
  test(`pictures at ${width} px in ${theme}`, async ({ page, world, httpRequests }) => {
    await page.emulateMedia({ colorScheme: theme });
    // Tall enough that the settings' own scroll box shows the whole page.
    await page.setViewportSize({ width, height: width < 600 ? 3200 : 2000 });
    mkdirSync(DIR!, { recursive: true });
    const shot = (name: string) => page.screenshot({ path: path.join(DIR!, `${name}-${theme}-${width}.png`) });
    await page.goto(`${world.url}/settings/models?lang=zh`);
    await expectWebRtc(page, httpRequests);
    await openOwnProviders(page);
    await expect(page.getByTestId('default-card').getByText('说话转成文字', { exact: true })).toBeVisible();
    await shot('page');

    await page.getByTestId('add-provider-button').click();
    const panel = page.getByTestId('add-provider');
    await panel.getByRole('button', { name: 'Anthropic', exact: true }).click();
    await panel.getByLabel('粘贴密钥').fill('test-only-marker');
    await panel.getByTestId('add-provider-confirm').click();
    const row = page.getByTestId('provider-row').filter({ hasText: 'Anthropic' }).first();
    await expect(row.getByTestId('key-set')).toBeVisible();
    const input = row.getByTestId('model-input').getByRole('textbox');
    await input.fill('claude-sonnet-5-5');
    await input.press('Enter');
    await expect(row.getByTestId('added-model')).toHaveCount(1);
    await shot('claude');

    const summary = page.getByTestId('chat-fallback-summary');
    await page.locator('[aria-controls="chat-fallback-editor"]').click();
    const add = page.getByRole('combobox', { name: '添加备用' });
    const sonnetOption = add.getByRole('option', { name: /^claude-sonnet-5-5 · / });
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

    await expect(row.getByTestId('model-input')).toBeVisible();
    await shot('claude-open');

    await openOwnProviders(page);
    await page.getByTestId('add-provider-button').click();
    await expect(page.getByTestId('add-provider')).toBeVisible();
    await shot('add-provider');
    await page.getByTestId('add-provider').getByRole('button', { name: '关闭' }).click();
    await page.locator('[data-testid="provider-row"][data-name="openai"]').getByRole('button').first().click();
    await shot('openai');
  });
}
