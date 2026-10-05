// A new chat: the message goes over the data channel, the fake model's
// scripted reply renders.
import {
  test, expect, expectWebRtc, expectNoHttpChat, expectScriptedCalls, send, shown,
  holdReplies, held, heldEvents, release,
} from '../support/world';

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
  // The previous session's rows (tests/fixtures/home/sessions) stay there.
  await expect(shown(page, 'Harbor Table. Alex rolled a 4.')).toHaveCount(0);

  await send(page, 'Say hello.');
  await expect(shown(page, 'Hello from the fake model.')).toHaveCount(1);
  await expect(shown(page, '[Ling]')).toHaveCount(1);
  await expect(shown(page, 'Say hello.')).toHaveCount(1);
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
  await expect(shown(page, 'Say hello.')).toHaveCount(1);
  await expect(shown(page, '[Ling]')).toHaveCount(1);
});
