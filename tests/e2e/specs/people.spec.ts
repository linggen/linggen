// Settings → People: the owner sets one person of the household — level, apps,
// a model and a daily limit — and it holds: the page shows the same after a
// reload, and that person's own view of the Mac is what was set.
// A second account's phone is paired as specs/person.spec.ts does it.
import { writeFileSync } from 'node:fs';
import type { APIRequestContext, Page } from '@playwright/test';
import { test, expect, expectWebRtc } from '../support/world';

const PHONE = { id: 'phone-friend', token: 'tok-friend-phone', account: 'u-friend' };
// Every app of tests/fixtures/home/skills: dice, abacus, fixture-options.
const KEEP = 'dice';

function pairFriendsPhone(home: string) {
  const row = {
    id: PHONE.id, name: "Friend's phone", secret: PHONE.token, created_at: 0, kind: 'phone',
    account: { id: PHONE.account, name: 'Friend' },
  };
  writeFileSync(`${home}/paired-devices.json`, JSON.stringify([row]));
}

async function friendIsKnown(world: { url: string }, request: APIRequestContext) {
  await expect
    .poll(async () => (await request.get(`${world.url}/api/pair/me`, { headers: { 'x-linggen-device': PHONE.token } })).status())
    .toBe(200);
}

async function openPeople(page: Page, url: string, httpRequests: string[]) {
  await page.goto(`${url}/settings`);
  await expectWebRtc(page, httpRequests);
  await page.getByRole('button', { name: 'People', exact: true }).click();
}

const friend = (page: Page) => page.locator('[data-testid="person"][data-person="u-friend"]');

test('the owner sets a person once, and it holds after a reload and in their own view', async ({ page, world, request, httpRequests }) => {
  pairFriendsPhone(world.linggenHome);
  await friendIsKnown(world, request);
  await openPeople(page, world.url, httpRequests);

  const card = friend(page);
  await expect(card).toBeVisible();
  await expect(card.getByText("Friend's phone · Phone")).toBeVisible();
  // Nothing set: read, every app, no model of this Mac, no limit.
  await expect(card.getByRole('button', { name: 'Read', exact: true })).toHaveAttribute('aria-pressed', 'true');
  await expect(card.getByText('No model of this Mac — they use their own')).toBeVisible();

  await card.getByRole('button', { name: 'Edit', exact: true }).click();
  await expect(card.getByRole('button', { name: 'Edit', exact: true })).toHaveAttribute('aria-pressed', 'true');

  // Only one app: switch the others off.
  const apps = card.locator('[data-app]');
  const names = await apps.evaluateAll((els) => els.map((el) => el.getAttribute('data-app') as string));
  expect(names).toContain(KEEP);
  for (const name of names.filter((n) => n !== KEEP)) {
    await card.locator(`[data-app="${name}"]`).click();
    await expect(card.locator(`[data-app="${name}"]`)).toHaveAttribute('aria-pressed', 'false');
  }

  const model = card.locator('[data-model]').first();
  const modelId = await model.getAttribute('data-model');
  await model.click();
  await expect(model).toHaveAttribute('aria-pressed', 'true');
  await expect(card.getByText('No model of this Mac — they use their own')).toHaveCount(0);

  await card.getByLabel('Daily limit').fill('200000');
  await card.getByLabel('Daily limit').press('Enter');
  await expect(card.getByTestId('used-today')).toHaveText('Used today: 0 of 200,000');

  // A reload shows what the engine kept.
  await page.reload();
  await expectWebRtc(page, httpRequests);
  await page.getByRole('button', { name: 'People', exact: true }).click();
  await expect(card.getByRole('button', { name: 'Edit', exact: true })).toHaveAttribute('aria-pressed', 'true');
  await expect(card.locator(`[data-app="${KEEP}"]`)).toHaveAttribute('aria-pressed', 'true');
  for (const name of names.filter((n) => n !== KEEP)) {
    await expect(card.locator(`[data-app="${name}"]`)).toHaveAttribute('aria-pressed', 'false');
  }
  await expect(card.locator(`[data-model="${modelId}"]`)).toHaveAttribute('aria-pressed', 'true');
  await expect(card.getByLabel('Daily limit')).toHaveValue('200000');

  // Their own view of this Mac: only the app they were given.
  const seen = await (await request.get(`${world.url}/api/skills`, { headers: { 'x-linggen-device': PHONE.token } })).json();
  expect((seen as { name: string }[]).map((s) => s.name)).toEqual([KEEP]);
});

test('an app the owner keeps is shown as kept in a person\'s list, not as a choice', async ({ page, world, request, httpRequests }) => {
  pairFriendsPhone(world.linggenHome);
  await friendIsKnown(world, request);
  await openPeople(page, world.url, httpRequests);

  await page.getByTestId('mine-only').locator(`[data-app="${KEEP}"]`).click();
  const mine = friend(page).locator(`[data-app="${KEEP}"]`);
  await expect(mine).toBeDisabled();
  await expect(mine).toContainText('kept for you');
  const seen = await (await request.get(`${world.url}/api/skills`, { headers: { 'x-linggen-device': PHONE.token } })).json();
  expect((seen as { name: string }[]).map((s) => s.name)).not.toContain(KEEP);
});

test("the page opened from the owner's other computer says people are set at this Mac, and offers no control", async ({ page, world, request, httpRequests }) => {
  const machine = { id: 'machine-studio', token: 'tok-studio', account: null };
  writeFileSync(`${world.linggenHome}/paired-devices.json`, JSON.stringify([
    { id: PHONE.id, name: "Friend's phone", secret: PHONE.token, created_at: 0, kind: 'phone', account: { id: PHONE.account, name: 'Friend' } },
    { id: machine.id, name: 'Studio', secret: machine.token, created_at: 0, kind: 'machine' },
  ]));
  await expect
    .poll(async () => (await request.get(`${world.url}/api/pair/me`, { headers: { 'x-linggen-device': machine.token } })).status())
    .toBe(200);
  // Its channel is the computer's own: admin, but not at this Mac.
  await page.route('**/api/rtc/whip', (route) =>
    route.continue({
      headers: { ...route.request().headers(), 'x-linggen-device': machine.token, 'x-linggen-actor-device': machine.id },
    }));
  await openPeople(page, world.url, httpRequests);
  // The engine answers the list (it is admin) and says the asker is not at this Mac.
  expect(await page.evaluate(async () => (await (await fetch('/api/people')).json()).at_this_mac)).toBe(false);
  await expect(page.getByText('People are set at this Mac itself, not from here.')).toBeVisible();
  await expect(page.locator('[data-testid="person"]')).toHaveCount(0);
  await expect(page.getByRole('button', { name: 'Edit', exact: true })).toHaveCount(0);
});
