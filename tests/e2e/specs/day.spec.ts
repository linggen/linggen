// Linggen's daily session (doc/one-ling-spec.md § Linggen's daily session), seen on the
// pages: with the setting on, two apps' chats are the day's session; a
// private app keeps a session of its own — its reply streams to its own page
// and to no other page — and the main page lists the day and folds another
// app's stretch to a line.
import { mkdirSync, writeFileSync } from 'node:fs';
import type { APIRequestContext, Page } from '@playwright/test';
import { test, expect, expectWebRtc, send, composer } from '../support/world';

const SECRET = 'LEDGER-ONLY reply: 120 shares.';
const ROLLED = 'Rolled a three.';

test.use({ scenario: { scripts: { ling: [{ text: SECRET }, { text: ROLLED }] } } });

/** A made-up private app, written into the world's home and reloaded. */
async function installLedger(request: APIRequestContext, url: string, home: string) {
  mkdirSync(`${home}/skills/ledger`, { recursive: true });
  writeFileSync(
    `${home}/skills/ledger/SKILL.md`,
    '---\nname: ledger\ndescription: A private test ledger.\nallowed-tools: []\nprivate-talk: true\n---\n\nYou keep the ledger.\n',
  );
  await request.post(`${url}/api/skills/reload`, { data: {} });
}

async function settingOn(request: APIRequestContext, url: string): Promise<string> {
  const config = await (await request.get(`${url}/api/config`)).json();
  config.agent.daily_session = true;
  await request.post(`${url}/api/config`, { data: config });
  return (await (await request.get(`${url}/api/sessions/day`)).json()).id;
}

const embed = (url: string, skill: string, session: string) => `${url}/embed?skill=${skill}&session=${session}&hide_toolbar=1`;

/** The session a new chat of `skill` is given. */
async function sessionFor(request: APIRequestContext, url: string, skill: string): Promise<string> {
  return (await (await request.post(`${url}/api/sessions`, { data: { title: skill, skill } })).json()).id;
}

/** Text a page shows — visible copies only. */
const seen = (page: Page, text: string) => page.getByText(text).filter({ visible: true });

/** A page opened beside the first: up once its chat box is. */
async function opened(page: Page, at: string) {
  await page.goto(at);
  await expect(composer(page)).toBeVisible();
}

test("a private app's reply reaches its own page alone; another app's stretch folds on the main page", async ({ page, context, world, request, httpRequests }) => {
  await installLedger(request, world.url, world.linggenHome);
  const day = await settingOn(request, world.url);
  expect(day).toMatch(/^sess-day-\d{4}-\d{2}-\d{2}$/);
  const own = await sessionFor(request, world.url, 'ledger');
  expect(own, 'the private app was given the day').not.toBe(day);

  const ledger = page;
  await page.goto(embed(world.url, 'ledger', own));
  await expectWebRtc(page, httpRequests);
  const dice = await context.newPage();
  const main = await context.newPage();
  await opened(dice, embed(world.url, 'dice', day));
  await opened(main, `${world.url}/?session=${day}`);

  await send(ledger, 'How many shares?');
  await expect(seen(ledger, SECRET)).toHaveCount(1);
  await send(dice, 'Roll for me');
  await expect(seen(dice, ROLLED)).toHaveCount(1);
  // The ledger's reply streamed to its own page alone.
  for (const other of [dice, main]) {
    await expect(seen(other, SECRET)).toHaveCount(0);
    await expect(seen(other, 'How many shares?')).toHaveCount(0);
  }

  await main.reload();
  await expect(composer(main)).toBeVisible();
  await expect(main.locator(`[data-testid="session-row"][data-session-id="${day}"]`)).toHaveCount(1);
  const fold = main.getByRole('button', { name: 'dice · 2 messages' });
  await expect(fold).toBeVisible();
  await expect(seen(main, SECRET)).toHaveCount(0);
  await expect(seen(main, ROLLED)).toHaveCount(0);
  await fold.click();
  await expect(seen(main, ROLLED)).toHaveCount(1);
});
