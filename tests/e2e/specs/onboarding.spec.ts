// First run: a fresh home (tests/fixtures/fresh — the engine's own defaults:
// no models of the person's, Linggen Cloud the default, signed out). What a
// new user sees. Never click a sign-in button here: the daemon would open the
// system browser.
// A real first install has no config/ at all; that world can't be hermetic
// yet — with no config the engine dials ling-mem at 127.0.0.1:9528 (the
// person's own) and has no other way to be told — so `fresh` is the default
// config plus only the test's ports.
import type { Page } from '@playwright/test';
import { test, expect, expectWebRtc, expectTurnOver, chat, send } from '../support/world';

test.use({ scenario: { fixture: 'fresh' } });

/** The chat's inline sign-in prompt (a turn that needs linggen.dev). */
const signInPrompt = (page: Page) => chat(page).getByTestId('auth-required');

test('a new install shows an empty chat and asks to sign in on the first message', async ({ page, world, httpRequests }) => {
  await page.goto(world.url);
  await expectWebRtc(page, httpRequests);
  await expect(page.getByTestId('session-list-empty')).toBeVisible();
  await expect(page.getByTestId('account-sign-in')).toBeVisible();
  // The built-in Linggen Cloud model is offered.
  await expect(page.getByText('deepseek-flash').first()).toBeVisible();

  await send(page, 'Hello?');
  const c = chat(page);
  await expect(c.getByText('Hello?').first()).toBeVisible();
  await expect(signInPrompt(page)).toHaveAttribute('data-provider', 'linggen');
  await expect(signInPrompt(page).getByRole('button', { name: 'Sign in', exact: true })).toBeVisible();
  await expect(c.getByText(/failed to send/i)).toHaveCount(0);
  // No model was called: there is none to call before signing in.
  expect(await world.calls()).toEqual([]);

  // A reload shows the session and its one sign-in prompt — checked on the
  // second load's own transport (the request list starts again on reload).
  await expectTurnOver(page);
  await page.reload();
  expect(httpRequests[0]).toBe('GET /');
  await expectWebRtc(page, httpRequests);
  await expect(signInPrompt(page)).toHaveCount(1);
  await expectTurnOver(page);
  await expect(signInPrompt(page)).toHaveCount(1);
});

// Found by this suite (2026-10-05): a failed turn's line was sent twice live
// (once unsaved by the run wrapper, once saved by the turn) — two sign-in
// prompts until a reload. One owner sends it now.
test('the first sign-in prompt shows once', async ({ page, world, httpRequests }) => {
  await page.goto(world.url);
  await expectWebRtc(page, httpRequests);
  await send(page, 'Hello?');
  await expect(signInPrompt(page)).toHaveCount(1);
  await expectTurnOver(page);
  await expect(signInPrompt(page)).toHaveCount(1);
});
