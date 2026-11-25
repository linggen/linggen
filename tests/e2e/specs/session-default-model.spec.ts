// The chat bar's model picker: "Default (…)" is the model the engine says the
// session runs on (`GET /api/sessions/default-model`, the turn's own rule),
// and that model is the one marked "default" in the list.
import { test, expect, expectWebRtc } from '../support/world';

const FERRY = 'sess-1790000002-c0ac7ed2';

test.use({ viewport: { width: 1280, height: 1000 } });

test('Default names the model the engine says this session runs on', async ({ page, world, httpRequests }) => {
  await page.goto(`${world.url}/?session=${FERRY}`);
  await expectWebRtc(page, httpRequests);
  const picker = page.getByTestId('session-model');
  await expect(picker).toBeVisible();
  const standing = picker.locator('option', { hasText: '— default' });
  await expect(standing).toHaveCount(1);
  const name = (await standing.textContent())!.replace(/ — default.*$/, '').replace(/ \(.*\)$/, '').trim();
  await expect(picker.locator('option[value=""]')).toHaveText(`Default (${name})`);
});
