// Prewritten sessions (tests/fixtures/home/sessions) as the chat page shows
// them — a run cut off before its reply, a shared table, a compacted one —
// and what the session list does with them: switch, rename, delete.
import type { Page } from '@playwright/test';
import { test, expect, expectWebRtc, expectScriptedCalls, expectTurnOver, chat, send, shown } from '../support/world';

const SHARED = 'sess-1790000001-5a1ed001';
const FERRY = 'sess-1790000002-c0ac7ed2';
const INTERRUPTED = 'sess-1790000003-1e7e5ed3';

const row = (page: Page, id: string) => page.locator(`[data-testid="session-row"][data-session-id="${id}"]`);

/** Open a session row's ⋯ menu and pick `item`. */
async function rowMenu(page: Page, id: string, item: 'Rename' | 'Delete') {
  await row(page, id).getByRole('button', { name: 'More actions' }).click();
  await row(page, id).getByRole('menuitem', { name: item }).click();
}

test('an interrupted run shows its tool calls and Interrupted', async ({ page, world, httpRequests }) => {
  await page.goto(`${world.url}/?session=${INTERRUPTED}`);
  await expectWebRtc(page, httpRequests);
  await expect(shown(page, 'List the notes here.')).toHaveCount(1);
  await expect(shown(page, 'Glob(*.md)')).toHaveCount(1);
  await expect(chat(page).getByTestId('interrupted-marker')).toHaveCount(1);
});

test('switching sessions shows each one its own rows', async ({ page, world, httpRequests }) => {
  await page.goto(`${world.url}/?session=${FERRY}`);
  await expectWebRtc(page, httpRequests);
  await expect(shown(page, 'Every thirty minutes on Sundays.')).toHaveCount(1);

  await row(page, INTERRUPTED).click();
  await expect(shown(page, 'List the notes here.')).toHaveCount(1);
  await expect(shown(page, 'Every thirty minutes on Sundays.')).toHaveCount(0);

  await row(page, FERRY).click();
  await expect(shown(page, 'Every thirty minutes on Sundays.')).toHaveCount(1);
  await expect(shown(page, 'List the notes here.')).toHaveCount(0);
});

// Found by this suite (2026-10-06), not fixed yet: Rename in the list is a
// silent no-op — sessionStore.renameSession returns before its PATCH while
// the store's selectedProjectRoot is '' (as here, on a fresh browser), and
// the row falls back to its old name. Expected to fail until it is fixed —
// then drop `test.fail`.
test('a session renamed in the list keeps its name after a reload', async ({ page, world, httpRequests }) => {
  test.fail();
  await page.goto(`${world.url}/?session=${FERRY}`);
  await expectWebRtc(page, httpRequests);
  await expect(row(page, FERRY)).toContainText('Ferry notes');

  await rowMenu(page, FERRY, 'Rename');
  const box = row(page, FERRY).getByRole('textbox');
  await box.fill('Sunday ferries');
  await box.press('Enter');
  await expect(row(page, FERRY)).toContainText('Sunday ferries', { timeout: 5_000 });

  await page.reload();
  await expectWebRtc(page, httpRequests);
  await expect(row(page, FERRY)).toContainText('Sunday ferries');
  await expect(page.getByTestId('session-list').getByText('Ferry notes')).toHaveCount(0);
});

test('a session deleted from the list is gone, after a reload too', async ({ page, world, httpRequests }) => {
  await page.goto(`${world.url}/?session=${INTERRUPTED}`);
  await expectWebRtc(page, httpRequests);
  await expect(row(page, FERRY)).toHaveCount(1);

  await rowMenu(page, FERRY, 'Delete');
  await page.getByRole('alertdialog').getByRole('button', { name: 'OK', exact: true }).click();
  await expect(row(page, FERRY)).toHaveCount(0);
  await expect(row(page, INTERRUPTED)).toHaveCount(1);

  await page.reload();
  await expectWebRtc(page, httpRequests);
  await expect(row(page, INTERRUPTED)).toHaveCount(1);
  await expect(row(page, FERRY)).toHaveCount(0);
});

test.describe('a shared table', () => {
  test.use({ scenario: { scripts: { yinyue: [{ text: 'Good luck, Alex.' }] } } });

  test('shows both members, and @银月 routes to Yinyue', async ({ page, world, httpRequests }) => {
    await page.goto(`${world.url}/?session=${SHARED}`);
    await expectWebRtc(page, httpRequests);
    // Both members speak in its thread.
    await expect(shown(page, '[Ling]')).toHaveCount(1);
    await expect(shown(page, 'Harbor Table. Alex rolled a 4.')).toHaveCount(1);
    await expect(shown(page, '[Yinyue]')).toHaveCount(1);
    await expect(shown(page, 'Four is a fine number.')).toHaveCount(1);

    await send(page, '@银月 wish me luck');
    await expect(shown(page, 'Good luck, Alex.')).toHaveCount(1);
    await expectTurnOver(page);
    await expect(shown(page, '[Yinyue]')).toHaveCount(2);
    // Only she was asked — not Ling.
    await expectScriptedCalls(world, ['yinyue']);
  });

  // Found by this suite (2026-10-05): the server keeps an addressed message
  // without its @name, the bubble held what was typed, and the two showed
  // until a reload. The bubble now matches its row by the id it was sent with.
  test('an @mention message shows once', async ({ page, world, httpRequests }) => {
    await page.goto(`${world.url}/?session=${SHARED}`);
    await expectWebRtc(page, httpRequests);
    await send(page, '@银月 wish me luck');
    await expect(shown(page, 'Good luck, Alex.')).toHaveCount(1);
    await expectTurnOver(page);
    await expect(shown(page, /wish me luck/)).toHaveCount(1);
    await expect(shown(page, 'Good luck, Alex.')).toHaveCount(1);
  });

  test('a reply is still there after a reload', async ({ page, world, httpRequests }) => {
    await page.goto(`${world.url}/?session=${SHARED}`);
    await expectWebRtc(page, httpRequests);
    await send(page, '@银月 wish me luck');
    await expect(shown(page, 'Good luck, Alex.')).toHaveCount(1);
    await expectTurnOver(page);

    await page.reload();
    await expectWebRtc(page, httpRequests);
    await expect(shown(page, 'Good luck, Alex.')).toHaveCount(1);
    await expect(shown(page, /wish me luck/)).toHaveCount(1);
    await expect(shown(page, '[Yinyue]')).toHaveCount(2);
    await expect(shown(page, 'Harbor Table. Alex rolled a 4.')).toHaveCount(1);
    await expectScriptedCalls(world, ['yinyue']);
  });
});
