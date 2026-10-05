// A new chat: the message goes over the data channel, the fake model's
// scripted reply renders.
import { test, expect, expectWebRtc, expectNoHttpChat, expectScriptedCalls, send, shown } from '../support/world';

test.use({ scenario: { scripts: { ling: [{ text: 'Hello from the fake model.' }] } } });

test('a sent message gets the scripted reply', async ({ page, world, httpRequests }) => {
  await page.goto(world.url);
  await expectWebRtc(page, httpRequests);
  await page.getByRole('button', { name: 'New chat', exact: true }).click();
  await send(page, 'Say hello.');

  await expect(shown(page, 'Say hello.')).toHaveCount(1);
  await expect(shown(page, 'Hello from the fake model.')).toHaveCount(1);
  await expect(shown(page, '[Ling]')).toHaveCount(1);
  // The new session is listed under its first message.
  await expect(page.getByText('Say hello.').first()).toBeVisible();
  expectNoHttpChat(httpRequests);
  await expectScriptedCalls(world, ['ling']);
});
