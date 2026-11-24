import path from 'node:path';
import { test, expect, expectWebRtc } from '../support/world';

test.use({ viewport: { width: 1280, height: 1000 }, colorScheme: 'light' });

test('default voice engine, live order and app model survive reload', async ({ page, world, httpRequests, request }) => {
  const response = await request.put(`${world.url}/api/credentials`, { data: { endpoints: [
    { provider: 'qwen', url: 'https://ap-southeast-1.maas.qwencloudapi.com/compatible-mode/v1', api_key: 'test-only-marker' },
    { provider: 'gemini', url: 'https://generativelanguage.googleapis.com/v1beta/openai', api_key: 'test-only-marker' },
  ] } });
  expect(response.ok()).toBeTruthy();
  await page.goto(`${world.url}/settings/models?lang=zh`);
  await expectWebRtc(page, httpRequests);
  const picks = [['speak', 'system'], ['live', 'google'], ['app-model-lesson', 'fake-eye']];
  for (const [id, value] of picks) {
    const select = page.getByTestId(`default-${id}`);
    await select.selectOption(value);
    await expect(select).toHaveValue(value);
    await expect(select).toBeEnabled();
  }
  await page.reload();
  await expectWebRtc(page, httpRequests);
  for (const [id, value] of picks) await expect(page.getByTestId(`default-${id}`)).toHaveValue(value);
  const card = page.getByTestId('default-card');
  await expect(card).toContainText('应用用的模型');
  await expect(card).toContainText('听写');
  const dir = process.env.MODELS_SHOTS_DIR;
  if (dir) await page.screenshot({ path: path.join(dir, 'uses-1280.png') });
});
