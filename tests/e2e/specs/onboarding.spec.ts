// First run: a fresh home (tests/fixtures/fresh — the engine's own defaults:
// no models of the person's, Linggen Cloud the default, signed out). What a
// new user sees. Never click a sign-in button here: the daemon would open the
// system browser.
import { test, expect, expectWebRtc, chat, send } from '../support/world';

test.use({ scenario: { fixture: 'fresh' } });

const signIn = (page: Parameters<typeof chat>[0]) =>
  chat(page).getByRole('button', { name: 'Sign in', exact: true });

test('a new install shows an empty chat and asks to sign in on the first message', async ({ page, world, httpRequests }) => {
  await page.goto(world.url);
  await expectWebRtc(page, httpRequests);
  await expect(page.getByText('No sessions yet')).toBeVisible();
  await expect(page.getByTitle('Sign in to linggen.dev')).toBeVisible();
  // The built-in Linggen Cloud model is offered.
  await expect(page.getByText('deepseek-flash').first()).toBeVisible();

  await send(page, 'Hello?');
  const c = chat(page);
  await expect(c.getByText('Hello?').first()).toBeVisible();
  await expect(c.getByText(/Not signed in to linggen\.dev/).first()).toBeVisible();
  await expect(signIn(page).first()).toBeVisible();
  await expect(c.getByText(/failed to send/i)).toHaveCount(0);
  // No model was called: there is none to call before signing in.
  expect(await world.calls()).toEqual([]);

  // A reload shows the session and its one sign-in prompt.
  await page.reload();
  await expectWebRtc(page, httpRequests);
  await expect(signIn(page)).toHaveCount(1);
});

// Found by this suite (2026-10-05): a failed turn's line was sent twice live
// (once unsaved by the run wrapper, once saved by the turn) — two sign-in
// prompts until a reload. One owner sends it now.
test('the first sign-in prompt shows once', async ({ page, world, httpRequests }) => {
  await page.goto(world.url);
  await expectWebRtc(page, httpRequests);
  await send(page, 'Hello?');
  await expect(signIn(page).first()).toBeVisible();
  await page.waitForTimeout(2_000);
  await expect(signIn(page)).toHaveCount(1, { timeout: 3_000 });
});
