// Settings → Models, the provider rows: paste a key once, add a model by
// typing (the provider's own list suggests; a full id + Enter adds it), remove
// it with ✕, remove a provider from ⋯ with a confirm, add one by hand (其他…),
// tick Linggen Cloud's models — every step on a scratch engine. Linggen Cloud's
// list is the scenario's `cloudModels` (the kept answer: the site is never
// called); no provider is called. The key is a marker, never a real one.
import type { Locator, Page } from '@playwright/test';
import { test, expect, expectWebRtc } from '../support/world';

/** The providers section is always open: wait for it. */
async function openOwnProviders(page: Page) {
  await page.getByTestId('own-providers').waitFor();
}

const KEY = 'test-only-marker';
const CLOUD = { updated: '2026-11-26', starter: 'stand-in-flash', models: [{ id: 'stand-in-flash', label: 'Stand-in Flash' }, { id: 'stand-in-max', label: 'Stand-in Max' }] };

test.use({ scenario: { cloudModels: CLOUD } });

const provider = (page: Page, name: string) => page.getByTestId('provider-row').filter({ hasText: name }).first();

async function openModels(page: Page, world: { url: string }, httpRequests: string[]) {
  await page.goto(`${world.url}/settings/models`);
  await expectWebRtc(page, httpRequests);
  await openOwnProviders(page);
  await expect(page.getByTestId('models-page')).toBeVisible();
}

/** Add Anthropic from the picker and paste the key once. */
async function addAnthropic(page: Page): Promise<Locator> {
  await openOwnProviders(page);
  await page.getByTestId('add-provider-button').click();
  const panel = page.getByTestId('add-provider');
  await panel.getByRole('button', { name: 'Anthropic', exact: true }).click();
  // A pick only selects: nothing is in the list until 添加 with a key.
  await expect(provider(page, 'Anthropic')).toHaveCount(0);
  await panel.getByLabel('Paste your key').fill(KEY);
  await panel.getByTestId('add-provider-confirm').click();
  const row = provider(page, 'Anthropic');
  await expect(row.getByTestId('key-set')).toBeVisible();
  return row;
}

test.describe('two names at one Qwen Cloud address', () => {
  const url = 'https://ap-southeast-1.maas.qwencloudapi.com/compatible-mode/v1';
  test.use({ scenario: {
    configAppend: `
[[models]]
id = "own-flash"
provider = "openai"
url = "${url}"
model = "deepseek-v4.1-flash"
`,
  } });

  test('one card shows the model, and can remove only the unused key', async ({ page, world, httpRequests, request }) => {
    const other = 'unused-test-key-marker';
    const filed = await request.put(`${world.url}/api/credentials`, { data: { endpoints: [
      { provider: 'openai', url, api_key: KEY },
      { provider: 'qwen', url: `${url}/`, api_key: other },
    ] } });
    expect(filed.ok()).toBe(true);
    await page.goto(`${world.url}/settings/models?lang=zh`);
    await expectWebRtc(page, httpRequests);
    await openOwnProviders(page);
    const row = page.getByTestId('provider-row').filter({ hasText: 'Qwen Cloud' });
    await expect(row).toHaveCount(1);
    await row.getByRole('button', { name: /Qwen Cloud/ }).click();
    await expect(row.getByTestId('added-model')).toHaveText(/deepseek-v4.1-flash/);
    await expect(row.getByTestId('live-model')).toHaveCount(0);
    await expect(row.getByTestId('chat-models-note')).toContainText('用于聊天');
    await expect(row.getByTestId('key-conflict')).toContainText('这个地址存了两把不同的密钥');
    await row.getByRole('button', { name: '保留模型使用的密钥' }).click();
    await expect(row.getByTestId('key-set')).toHaveText('已设置密钥');
    await expect(row.getByTestId('key-conflict')).toBeHidden();
    await expect(row).not.toContainText(KEY);
    await expect(row).not.toContainText(other);
  });
});

test('a provider is added from the list with one key; a model is typed, added with Enter, and removed with ✕', async ({ page, world, httpRequests }) => {
  await openModels(page, world, httpRequests);
  const row = await addAnthropic(page);
  // Nothing is ticked for a new key: the person adds what they want.
  await expect(row.getByTestId('added-model')).toHaveCount(0);
  // The provider's list cannot be read here: a full id + Enter adds it anyway.
  const input = row.getByTestId('model-input').getByRole('textbox');
  await input.fill('claude-typed-test');
  await input.press('Enter');
  await expect(row.getByTestId('added-model')).toContainText('claude-typed-test');
  await row.getByRole('button', { name: 'Remove claude-typed-test' }).click();
  await expect(row.getByTestId('added-model')).toHaveCount(0);
  await expect(page.getByTestId('models-page')).not.toContainText(KEY);
});

test('a provider is removed from its ⋯ menu after a confirm that names the default it holds', async ({ page, world, httpRequests }) => {
  await openModels(page, world, httpRequests);
  const row = await addAnthropic(page);
  const input = row.getByTestId('model-input').getByRole('textbox');
  await input.fill('claude-typed-test');
  await input.press('Enter');
  await row.getByTestId('added-model').getByRole('button', { name: 'Make default' }).click();
  await row.getByRole('button', { name: 'Options' }).click();
  await page.getByRole('menuitem', { name: 'Remove this provider' }).click();
  await expect(page.getByText('It holds your default chat model.', { exact: false })).toBeVisible();
  await page.getByRole('alertdialog').getByRole('button', { name: 'OK' }).click();
  await expect(page.getByTestId('provider-row').filter({ hasText: 'Anthropic' })).toHaveCount(0);
});

test('another provider is typed: a name, an address, its API style and a key', async ({ page, world, httpRequests }) => {
  await openModels(page, world, httpRequests);
  await page.getByTestId('add-provider-button').click();
  await page.getByTestId('add-other').click();
  const form = page.getByTestId('add-provider-form');
  await form.getByLabel('Name').fill('My gateway');
  await form.getByLabel('Address (https://…)').fill('https://gateway.example.test/v1');
  await form.getByLabel('API style').selectOption('claude');
  await form.getByLabel('Paste your key').fill(KEY);
  await form.getByRole('button', { name: 'Add', exact: true }).click();
  const row = provider(page, 'gateway.example.test');
  await expect(row.getByTestId('key-set')).toBeVisible();
  await expect(row.getByTestId('provider-address')).toBeVisible();
});

test("Linggen Cloud's models are ticked like rows and offered on the default card", async ({ page, world, httpRequests }) => {
  await page.goto(`${world.url}/settings/models`);
  await expectWebRtc(page, httpRequests);
  const cloud = page.getByTestId('provider-row').filter({ hasText: 'Linggen Cloud' });
  await cloud.getByRole('button', { name: /Linggen Cloud/ }).click();
  const max = cloud.getByTestId('cloud-model').filter({ hasText: 'Stand-in Max' }).getByRole('checkbox');
  await expect(max).not.toBeChecked();
  await max.click();
  await expect(max).toBeChecked();
  await page.reload();
  await expectWebRtc(page, httpRequests);
  await page.getByTestId('provider-row').filter({ hasText: 'Linggen Cloud' }).getByRole('button', { name: /Linggen Cloud/ }).click();
  await expect(page.getByTestId('cloud-model').filter({ hasText: 'Stand-in Max' }).getByRole('checkbox')).toBeChecked();
});

test('a key row has no live-voice line and says its models are for chat', async ({ page, world, httpRequests }) => {
  await page.goto(`${world.url}/settings/models?lang=zh`);
  await expectWebRtc(page, httpRequests);
  await openOwnProviders(page);
  const row = await addAnthropic(page);
  await expect(row.getByTestId('live-model')).toHaveCount(0);
  await expect(row.getByTestId('chat-models-note')).toContainText('这里添加的模型用于聊天，请选文字模型。实时语音和朗读由 Linggen Cloud 提供。');
});

test.describe('a narrow window', () => {
  test.use({ viewport: { width: 420, height: 900 } });

  test('rows stack and nothing scrolls sideways', async ({ page, world, httpRequests }) => {
    await openModels(page, world, httpRequests);
    await openOwnProviders(page);
    const row = await addAnthropic(page);
    await expect(row.getByTestId('model-input')).toBeVisible();
    // The page, and every box around it up to the window, is no wider than what it shows.
    const wider = await page.getByTestId('models-page').evaluate((el) => {
      const out: string[] = [];
      for (let n: Element | null = el; n; n = n.parentElement) {
        if (n.scrollWidth > n.clientWidth + 1) out.push(`${n.tagName}.${(n as HTMLElement).className.toString().slice(0, 40)} ${n.scrollWidth}>${n.clientWidth}`);
      }
      return out;
    });
    expect(wider).toEqual([]);
  });
});
