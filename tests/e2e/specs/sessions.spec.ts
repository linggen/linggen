// Prewritten sessions (tests/fixtures/home/sessions) as the chat page shows
// them: a run cut off before its reply, and a shared table.
import { test, expect, expectWebRtc, expectNoHttpChat, expectScriptedCalls, send, shown } from '../support/world';

const SHARED = 'sess-1790000001-5a1ed001';
const INTERRUPTED = 'sess-1790000003-1e7e5ed3';

test('an interrupted run shows its tool calls and Interrupted', async ({ page, world, httpRequests }) => {
  await page.goto(`${world.url}/?session=${INTERRUPTED}`);
  await expectWebRtc(page, httpRequests);
  await expect(shown(page, 'List the notes here.')).toHaveCount(1);
  await expect(shown(page, 'Glob(*.md)')).toHaveCount(1);
  await expect(shown(page, /^⎿?\s*Interrupted$/)).toHaveCount(1);
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
    await expect(shown(page, '[Yinyue]')).toHaveCount(2);
    expectNoHttpChat(httpRequests);
    // Only she was asked — not Ling.
    await expectScriptedCalls(world, ['yinyue']);
  });

  // KNOWN BUG (found by this suite, 2026-10-05): the server keeps an
  // addressed message without its @name ("wish me luck"), the optimistic
  // bubble keeps it ("@银月 wish me luck"); mergeChatMessages matches user
  // rows by exact text, so both show until a reload. Remove `test.fail`
  // once fixed.
  test('an @mention message shows once', async ({ page, world, httpRequests }) => {
    test.fail();
    await page.goto(`${world.url}/?session=${SHARED}`);
    await expectWebRtc(page, httpRequests);
    await send(page, '@银月 wish me luck');
    await expect(shown(page, 'Good luck, Alex.')).toHaveCount(1);
    await expect(shown(page, /wish me luck/)).toHaveCount(1, { timeout: 3_000 });
  });
});
