// Linggen's daily session (doc/one-ling-spec.md § Linggen's daily session), seen on the
// pages: with the setting on, two apps' chats are the day's session; a
// private app keeps a session of its own — its reply streams to its own page
// and to no other page — and the main page lists the day and folds another
// app's stretch to a line.
import { mkdirSync, realpathSync, writeFileSync } from 'node:fs';
import type { APIRequestContext, Page } from '@playwright/test';
import { test, expect, expectWebRtc, expectTurnOver, expectScriptedCalls, chat, send, shown, composer } from '../support/world';

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

// The fixes of the daily session's last day, seen on real pages (setting on).

/** An app's page with its toolbar (its "Running" mark, its session list). */
const embedWithToolbar = (url: string, skill: string, session: string) => `${url}/embed?skill=${skill}&session=${session}`;

/** The path as the shell prints it (the world's folder may sit behind a link). */
function real(path: string): string {
  try {
    return realpathSync(path);
  } catch {
    return path;
  }
}

/** The session bar opened: its Clear and System Prompt buttons show. */
async function openBar(page: Page) {
  const bar = page.getByTestId('session-bar');
  await bar.locator('summary span').first().click();
  await expect(bar).toHaveAttribute('open', '');
}

/** A `!` command typed at a page, and the output it printed. The page
 *  shows the command as code (`$ cmd`) and then its output as code. */
async function bang(page: Page, command: string): Promise<string> {
  const blocks = page.locator('code').filter({ visible: true });
  const before = await blocks.count();
  await send(page, `!${command}`);
  await expect.poll(() => blocks.count()).toBeGreaterThan(before);
  await expect.poll(async () => (await blocks.last().innerText()).trim()).not.toMatch(/^\$ /);
  return (await blocks.last().innerText()).trim();
}

/** What a page put on the clipboard (the write is kept in `__copied`). */
function watchClipboard() {
  (window as unknown as { __copied: string }).__copied = '';
  Clipboard.prototype.writeText = async function (text: string) {
    (window as unknown as { __copied: string }).__copied = text;
  };
}
const copied = (page: Page) => page.evaluate(() => (window as unknown as { __copied: string }).__copied);

test('a ! command at an app\'s page stands in the app\'s folder, and its cd moves no other page', async ({ page, context, world, request, httpRequests }) => {
  const day = await settingOn(request, world.url);
  const dice = page;
  await dice.goto(embed(world.url, 'dice', day));
  await expectWebRtc(dice, httpRequests);
  const main = await context.newPage();
  await opened(main, `${world.url}/?session=${day}`);

  const folder = real(`${world.linggenHome}/skills/dice`);
  expect(await bang(dice, 'pwd')).toBe(folder);
  const workspace = await bang(main, 'pwd');
  expect(workspace).toBe(real(`${world.linggenHome}/workspace`));
  expect(workspace).not.toBe(folder);

  // A cd at the app's page is that page's alone.
  expect(await bang(dice, 'cd scripts && pwd')).toBe(`${folder}/scripts`);
  expect(await bang(dice, 'pwd')).toBe(`${folder}/scripts`);
  expect(await bang(main, 'pwd')).toBe(workspace);
});

test('Copy system prompt at an app\'s page gives that app\'s prompt; at the main page it does not', async ({ page, context, world, request, httpRequests }) => {
  await context.addInitScript(watchClipboard);
  const day = await settingOn(request, world.url);
  const dice = page;
  await dice.goto(embed(world.url, 'dice', day));
  await expectWebRtc(dice, httpRequests);
  const main = await context.newPage();
  await opened(main, `${world.url}/?session=${day}`);
  const DICE_PROMPT = 'You run the dice table.';

  await openBar(dice);
  await dice.getByTestId('copy-system-prompt').click();
  await expect.poll(() => copied(dice)).toContain(DICE_PROMPT);

  await openBar(main);
  await main.getByTestId('copy-system-prompt').click();
  await expect.poll(() => copied(main)).toContain('You are Ling');
  expect(await copied(main)).not.toContain(DICE_PROMPT);
});

test.describe('a turn running from the main page', () => {
  test.use({
    scenario: {
      scripts: { ling: [{ tool: 'Bash', args: { cmd: 'echo started > started.txt; sleep 8' } }, { text: 'Slept well.' }] },
    },
  });

  test('Clear at an app\'s page does not stop it', async ({ page, context, world, request, httpRequests }) => {
    const day = await settingOn(request, world.url);
    const main = page;
    await main.goto(`${world.url}/?session=${day}`);
    await expectWebRtc(main, httpRequests);
    await expect(composer(main)).toBeVisible();
    const dice = await context.newPage();
    await opened(dice, embedWithToolbar(world.url, 'dice', day));
    await openBar(dice);

    await send(main, 'Take a nap.');
    const c = chat(main);
    await c.getByTestId('permission-card').getByTestId('permission-directory').last().click();
    const bash = c.locator('[data-testid="tool-call"][data-tool="Bash"]');
    await expect(bash).toHaveAttribute('data-status', 'running');
    // The app's page knows a run is going on in the day — Clear is pressed with it running.
    await expect(dice.getByText('Running', { exact: true })).toBeVisible();
    await dice.getByTestId('clear-chat').click();

    await expect(shown(main, 'Slept well.')).toHaveCount(1, { timeout: 30_000 });
    await expectTurnOver(main);
    await expect(c.getByTestId('interrupted-marker')).toHaveCount(0);
    await expect(bash).toHaveAttribute('data-status', 'done');
    await expectScriptedCalls(world, ['ling', 'ling']);
  });
});

test.describe('a model chosen in the picker', () => {
  test.use({ scenario: { scripts: { ling: [{ text: 'Rolled.' }, { text: 'Home.' }] } } });

  test('is not changed by a message from an app\'s page opened before the choice', async ({ page, context, world, request, httpRequests }) => {
    const day = await settingOn(request, world.url);
    // The app's page pins its model (?model=) and is opened first.
    const dice = page;
    await dice.goto(`${embed(world.url, 'dice', day)}&model=fake-ling`);
    await expectWebRtc(dice, httpRequests);
    const main = await context.newPage();
    await opened(main, `${world.url}/?session=${day}`);

    const picker = main.getByTestId('session-model');
    await picker.selectOption('fake-eye');
    await expect(picker).toHaveValue('fake-eye');
    // The choice is kept by the engine before the other page speaks.
    const kept = async () => {
      const list = (await (await request.get(`${world.url}/api/skill-sessions?skill=dice`)).json()).sessions;
      return list.find((s: { id: string }) => s.id === day)?.agents?.find((a: { id: string }) => a.id === 'ling')?.model ?? null;
    };
    await expect.poll(kept).toBe('fake-eye');

    await send(dice, 'Roll for me');
    await expect(seen(dice, 'Rolled.')).toHaveCount(1);
    await expect(picker).toHaveValue('fake-eye');
    expect(await kept()).toBe('fake-eye');
    await main.reload();
    await expect(composer(main)).toBeVisible();
    await expect(main.getByTestId('session-model')).toHaveValue('fake-eye');

    // The main page's next turn runs on the picker's model; the app page's ran on its own.
    await send(main, 'Back on the main page');
    await expect(shown(main, 'Home.')).toHaveCount(1);
    await expectTurnOver(main);
    expect((await world.calls()).map((call) => call.model)).toEqual(['fake-ling-1', 'fake-eye-1']);
  });
});

test.describe('an app\'s page opened with the setting on', () => {
  test.use({ scenario: { scripts: { ling: [{ text: ROLLED }] } } });

  test('lists the day, and what is typed there folds to one line on the main page', async ({ page, context, world, request, httpRequests }) => {
    const day = await settingOn(request, world.url);
    // A new chat of the app is given the day.
    expect(await sessionFor(request, world.url, 'dice')).toBe(day);
    const dice = page;
    await dice.goto(embedWithToolbar(world.url, 'dice', day));
    await expectWebRtc(dice, httpRequests);
    await expect(composer(dice)).toBeVisible();
    // Its session list is on the day.
    const list = dice.locator('select').nth(1);
    await expect(list).toHaveValue(day);
    await expect(list.locator('option').filter({ hasText: /^Linggen · / })).toHaveCount(1);

    await send(dice, 'Roll for me');
    await expect(seen(dice, ROLLED)).toHaveCount(1);

    const main = await context.newPage();
    await opened(main, `${world.url}/?session=${day}`);
    await expect(main.locator(`[data-testid="session-row"][data-session-id="${day}"]`)).toHaveCount(1);
    const fold = main.getByRole('button', { name: 'dice · 2 messages' });
    await expect(fold).toBeVisible();
    await expect(seen(main, ROLLED)).toHaveCount(0);
    await fold.click();
    await expect(seen(main, ROLLED)).toHaveCount(1);
    await expect(seen(main, 'Roll for me')).toHaveCount(1);
  });
});

test.describe('Yinyue at home in the day', () => {
  test.use({ scenario: { scripts: { yinyue: [{ text: 'Good luck, Alex.' }] } } });

  test('answers @银月 in the day, and her line is there after a reload', async ({ page, world, request, httpRequests }) => {
    const day = await settingOn(request, world.url);
    await page.goto(`${world.url}/?session=${day}`);
    await expectWebRtc(page, httpRequests);
    await send(page, '@银月 wish me luck');
    await expect(shown(page, 'Good luck, Alex.')).toHaveCount(1);
    await expectTurnOver(page);
    await expectScriptedCalls(world, ['yinyue']);

    await page.reload();
    await expectWebRtc(page, httpRequests);
    await expect(shown(page, 'Good luck, Alex.')).toHaveCount(1);
    await expect(shown(page, '[Yinyue]')).toHaveCount(1);
    await expect(page.locator(`[data-testid="session-row"][data-session-id="${day}"]`)).toHaveCount(1);
  });
});
