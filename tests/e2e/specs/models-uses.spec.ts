import path from 'node:path';
import { test, expect, expectWebRtc } from '../support/world';

test.use({ viewport: { width: 1280, height: 1000 }, colorScheme: 'light' });

test('default voice engine survives reload', async ({ page, world, httpRequests }) => {
  await page.goto(`${world.url}/settings/models?lang=zh`);
  await expectWebRtc(page, httpRequests);
  const card = page.getByTestId('default-card');
  await expect(card).not.toContainText('实时语音');
  // A Mac hears with Apple's own recognition (one fact, no download); anywhere else, Whisper.
  await expect(card.getByText(process.platform === 'darwin' ? '苹果系统识别' : 'Whisper large-v3 turbo', { exact: true })).toBeVisible();
  // 画图 is hidden from people: no drawing line on the card.
  await expect(card).not.toContainText('画图');
  await expect(card).not.toContainText('FLUX');
  await expect(page.getByTestId('default-speak').locator('option[value="system"]')).toHaveText('macOS 系统声音 · 本机 · 免费');
  const speak = page.getByTestId('default-speak');
  await speak.selectOption('system');
  await expect(speak).toHaveValue('system');
  await expect(speak).toBeEnabled();
  await page.reload();
  await expectWebRtc(page, httpRequests);
  await expect(speak).toHaveValue('system');
  await expect(card).not.toContainText('应用用的模型');
  await expect(card).toContainText('听写');
  const dir = process.env.MODELS_SHOTS_DIR;
  if (dir) await page.screenshot({ path: path.join(dir, 'uses-1280.png') });
});

test('chat fallbacks are chosen by hand and survive reload', async ({ page, world, httpRequests, request }) => {
  const endpoint = `${world.url}/api/uses`;
  await page.goto(`${world.url}/settings/models?lang=zh`);
  await expectWebRtc(page, httpRequests);
  const summary = page.getByTestId('chat-fallback-summary');
  await expect(summary).toHaveText('无备用');
  await page.locator('[aria-controls="chat-fallback-editor"]').click();
  const add = page.getByRole('combobox', { name: '添加备用' });
  const rows = page.getByTestId('chat-fallback');
  const eye = rows.and(page.locator('[data-model="fake-eye"]'));
  await expect(add.locator('option[value="fake-ling"]')).toHaveCount(0);
  await add.selectOption('fake-eye');
  await expect(rows).toHaveCount(1);
  await add.selectOption('fake-sol');
  await expect(rows).toHaveCount(2);
  await expect(add.locator('option[value="fake-eye"]')).toHaveCount(0);
  await eye.getByRole('button', { name: /^下移 / }).click();
  const chain = async () => (await (await request.get(endpoint)).json()).chain;
  await expect.poll(chain).toEqual(['fake-ling', 'fake-sol', 'fake-eye']);
  await expect(page.getByText('备用只在开启自动换模型时用', { exact: true })).toBeVisible();
  const toggle = page.getByRole('switch', { name: '自动换模型' });
  await toggle.click();
  await expect(toggle).toBeChecked();
  await expect(toggle).toBeEnabled();

  // Removing the provider model doesn't silently edit the user's fallback list.
  const configUrl = `${world.url}/api/config`;
  const config = await (await request.get(configUrl)).json();
  config.models = config.models.filter((model: { id: string }) => model.id !== 'fake-sol');
  expect((await request.post(configUrl, { data: config })).ok()).toBeTruthy();
  await page.reload();
  await expectWebRtc(page, httpRequests);
  await expect(summary).toContainText('→');
  await expect(toggle).toBeChecked();
  await page.locator('[aria-controls="chat-fallback-editor"]').click();
  await expect(rows.nth(0)).toHaveAttribute('data-model', 'fake-sol');
  await expect(rows.nth(0)).toContainText('模型已不在配置中');
  await expect(rows.nth(1)).toHaveAttribute('data-model', 'fake-eye');
  await rows.nth(0).getByRole('button', { name: /^移除 / }).click();
  await expect(rows).toHaveCount(1);
  await expect.poll(chain).toEqual(['fake-ling', 'fake-eye']);
  await page.getByTestId('default-chat').selectOption('fake-eye');
  await expect(summary).toHaveText('无备用');
  await expect.poll(chain).toEqual(['fake-eye']);
});
