// The page loads from the test engine and the UI's real WebRTC transport
// comes up: signaling over /api/rtc, then a connected peer connection.
import { test, expect, expectWebRtc, chat } from '../support/world';

test('page loads and the WebRTC transport connects', async ({ page, world, httpRequests }) => {
  await page.goto(world.url);
  await expect(page.getByRole('heading', { name: 'Linggen' })).toBeVisible();
  await expectWebRtc(page, httpRequests);
  // The fixture home's sessions are listed (read over the data channel).
  await expect(page.getByText('Ferry notes')).toBeVisible();
  await expect(page.getByText('Cut-off run')).toBeVisible();
  await expect(chat(page).getByPlaceholder(/^Message/)).toBeVisible();
});
