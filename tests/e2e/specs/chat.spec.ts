// A new chat: the message goes over the data channel, the fake model's
// scripted reply renders.
import {
  test, expect, expectWebRtc, expectScriptedCalls, expectTurnOver, chat, send, shown,
  holdReplies, held, heldEvents, release,
} from '../support/world';
import type { Page } from '@playwright/test';

test.use({ scenario: { scripts: { ling: [{ text: 'Hello from the fake model.' }] } } });

test('a sent message gets the scripted reply', async ({ page, world, httpRequests }) => {
  await page.goto(world.url);
  await expectWebRtc(page, httpRequests);
  await page.getByRole('button', { name: 'New chat', exact: true }).click();
  await send(page, 'Say hello.');

  await expect(shown(page, 'Hello from the fake model.')).toHaveCount(1);
  await expectTurnOver(page);
  await expect(shown(page, 'Say hello.')).toHaveCount(1);
  await expect(shown(page, 'Hello from the fake model.')).toHaveCount(1);
  await expect(shown(page, '[Ling]')).toHaveCount(1);
  // The new session is listed under its first message.
  await expect(page.getByTestId('session-row').filter({ hasText: 'Say hello.' })).toHaveCount(1);
  await expectScriptedCalls(world, ['ling']);
});

// Found by this suite (2026-10-05, ~2 in 10 runs), two ways a new chat
// showed the session before it:

// 1. The page's load of the session it opened on answered after New chat,
//    and its rows landed in the new chat. Here that load is held until the
//    new chat's own load is back.
test('a late load of the previous session never lands in a new chat', async ({ page, world, httpRequests }) => {
  await page.addInitScript(holdReplies, { asked: [{ method: 'GET', path: '/api/workspace/state' }] });
  await page.goto(world.url);
  await expectWebRtc(page, httpRequests);
  await expect.poll(() => held(page)).not.toEqual([]);
  const [previous] = await held(page);

  await page.getByRole('button', { name: 'New chat', exact: true }).click();
  await expect.poll(async () => (await held(page)).some((s) => s !== previous)).toBe(true);
  const fresh = (await held(page)).find((s) => s !== previous)!;
  await release(page, [fresh, previous]);

  await send(page, 'Say hello.');
  await expect(shown(page, 'Hello from the fake model.')).toHaveCount(1);
  // The late load was handled long before this turn ended: had its rows
  // (tests/fixtures/home/sessions) landed, they would show now.
  await expectTurnOver(page);
  await expect(shown(page, 'Hello from the fake model.')).toHaveCount(1);
  await expect(shown(page, '[Ling]')).toHaveCount(1);
  await expect(shown(page, 'Say hello.')).toHaveCount(1);
  await expect(shown(page, 'Harbor Table. Alex rolled a 4.')).toHaveCount(0);
  await expect(shown(page, 'Four is a fine number.')).toHaveCount(0);
});

// 2. A message sent while New chat was still being made went to the session
//    before it — the chat then moved to the new, empty one. Here the new
//    session's creation is held while the message is sent.
test('a message sent while New chat is being made goes to the new chat', async ({ page, world, httpRequests }) => {
  await page.addInitScript(holdReplies, { asked: [{ method: 'POST', path: '/api/sessions' }] });
  await page.goto(world.url);
  await expectWebRtc(page, httpRequests);
  await expect(shown(page, 'Harbor Table. Alex rolled a 4.')).toHaveCount(1);

  await page.getByRole('button', { name: 'New chat', exact: true }).click();
  await expect.poll(() => held(page)).toEqual(['/api/sessions']);
  await send(page, 'Say hello.');
  await release(page);

  await expect(shown(page, 'Hello from the fake model.')).toHaveCount(1);
  await expectTurnOver(page);
  await expect(shown(page, 'Hello from the fake model.')).toHaveCount(1);
  await expect(shown(page, 'Say hello.')).toHaveCount(1);
  await expect(shown(page, '[Ling]')).toHaveCount(1);
  await expect(shown(page, 'Harbor Table. Alex rolled a 4.')).toHaveCount(0);
  await expectScriptedCalls(world, ['ling']);
});

// 3. The engine sent a turn's TurnComplete (and Idle) straight to the page,
//    while the run's own last events (its text block, its message) were still
//    queued: the page finalized the reply, the late block opened a second
//    [Ling] bubble and the message filled it. Here the new chat's events are
//    held until the turn is over, and their order is read.
test('a turn ends after its reply', async ({ page, world, httpRequests }) => {
  await page.addInitScript(holdReplies, { events: true });
  await page.goto(world.url);
  await expectWebRtc(page, httpRequests);
  await page.getByRole('button', { name: 'New chat', exact: true }).click();
  await send(page, 'Say hello.');
  await expect.poll(() => heldEvents(page)).toContain('turn_complete');

  const events = await heldEvents(page);
  const ended = events.indexOf('turn_complete');
  expect(events.indexOf('message:Hello from the fake model.')).toBeGreaterThanOrEqual(0);
  expect(events.slice(ended)).not.toContain('message:Hello from the fake model.');
  expect(events.slice(ended)).not.toContain('content_block');
  expect(events.slice(ended)).not.toContain('text_segment');

  await release(page);
  await expect(shown(page, 'Hello from the fake model.')).toHaveCount(1);
  await expectTurnOver(page);
  await expect(shown(page, 'Hello from the fake model.')).toHaveCount(1);
  await expect(shown(page, 'Say hello.')).toHaveCount(1);
  await expect(shown(page, '[Ling]')).toHaveCount(1);
});

test.describe('a live run', () => {
  test.use({
    scenario: {
      scripts: {
        ling: [{ tool: 'Bash', args: { cmd: 'sleep 120', timeout_ms: 180_000 } }, { text: 'Back again.' }],
      },
    },
  });

  /** A new chat whose run sits in a two-minute Bash, stopped from the box. */
  async function stopLiveRun(page: Page) {
    await page.getByRole('button', { name: 'New chat', exact: true }).click();
    await send(page, 'Wait a while.');
    const c = chat(page);
    await c.getByTestId('permission-card').getByRole('button', { name: /^Allow once/ }).click();
    await expect(c.locator('[data-testid="tool-call"][data-tool="Bash"]')).toHaveAttribute('data-status', 'running');
    await expect(c.getByTestId('chat-input')).toHaveAttribute('data-busy', 'true');
    await c.getByRole('button', { name: 'Stop agent' }).click();
    await expectTurnOver(page);
  }

  test('Stop ends it, marks it interrupted, and the chat goes on', async ({ page, world, httpRequests }) => {
    await page.goto(world.url);
    await expectWebRtc(page, httpRequests);
    await stopLiveRun(page);

    const c = chat(page);
    await expect(c.getByTestId('interrupted-marker')).toHaveCount(1);
    await expect(c.getByRole('button', { name: 'Stop agent' })).toHaveCount(0);
    // The box takes the next message, and it runs.
    await send(page, 'Still there?');
    await expect(shown(page, 'Back again.')).toHaveCount(1);
    await expectTurnOver(page);
    await expectScriptedCalls(world, ['ling', 'ling']);
  });

  // Found by this suite (2026-10-06): live, the stopped Bash still showed
  // running and a second, empty [Ling] bubble opened; a reload showed one
  // bubble and the call finished. The Stop request said Idle before the run
  // had ended, so the run's own last events landed after the page closed
  // the turn; now only the run's own end says Idle.
  test('a stopped run shows live as after a reload', async ({ page, world, httpRequests }) => {
    await page.goto(world.url);
    await expectWebRtc(page, httpRequests);
    await stopLiveRun(page);

    const c = chat(page);
    await expect(c.locator('[data-testid="tool-call"][data-tool="Bash"]')).not.toHaveAttribute('data-status', 'running', { timeout: 5_000 });
    await expect(shown(page, '[Ling]')).toHaveCount(1, { timeout: 5_000 });
  });
});
