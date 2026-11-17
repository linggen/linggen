// A day is its own person's (doc/one-ling-spec.md § One session a day). A
// phone paired to this Mac and signed in to another account reaches it the
// way a phone on the LAN does: its channel opened with the phone's token,
// every request riding that channel. It is given its own person's day, a
// message it sends naming the builder's day lands in its own, and nothing
// of the builder's day streams to it.
import { readFileSync, writeFileSync } from 'node:fs';
import { test, expect, expectWebRtc, send, composer } from '../support/world';

const PHONE = { id: 'phone-friend', token: 'tok-friend-phone', account: 'u-friend' };

test.use({ scenario: { scripts: { ling: [{ text: 'Rolled for the friend.' }, { text: 'BUILDER-ONLY reply.' }] } } });

/** The friend's phone, paired to this Mac and signed in to their account. */
function pairFriendsPhone(home: string) {
  const row = {
    id: PHONE.id, name: "Friend's phone", secret: PHONE.token, created_at: 0, kind: 'phone',
    account: { id: PHONE.account, name: 'Friend' },
  };
  writeFileSync(`${home}/paired-devices.json`, JSON.stringify([row]));
}

const rows = (home: string, sid: string) => {
  try {
    return readFileSync(`${home}/sessions/${sid}/messages.jsonl`, 'utf8');
  } catch {
    return '';
  }
};

test("another person's paired phone, over the channel, gets its own day and never the builder's", async ({ page, world, request, httpRequests }) => {
  pairFriendsPhone(world.linggenHome);
  // The Mac knows the phone once its token is answered.
  await expect
    .poll(async () => (await request.get(`${world.url}/api/pair/me`, { headers: { 'x-linggen-device': PHONE.token } })).status())
    .toBe(200);
  const config = await (await request.get(`${world.url}/api/config`)).json();
  config.agent.one_session_a_day = true;
  await request.post(`${world.url}/api/config`, { data: config });
  const mine: string = (await (await request.get(`${world.url}/api/sessions/day`)).json()).id;

  // The channel is opened as the phone's: its token, from off this Mac.
  await page.route('**/api/rtc/whip', (route) =>
    route.continue({
      headers: { ...route.request().headers(), 'x-linggen-device': PHONE.token, 'x-linggen-actor-device': PHONE.id },
    }));
  await page.goto(`${world.url}/embed?skill=dice&session=${mine}&hide_toolbar=1`);
  await expectWebRtc(page, httpRequests);
  await expect(composer(page)).toBeVisible();

  const theirs: string = await page.evaluate(async () => (await (await fetch('/api/sessions/day')).json()).id);
  expect(theirs, "the phone was handed the builder's day").not.toBe(mine);
  expect(theirs).toMatch(/^sess-day-\d{4}-\d{2}-\d{2}-p-[0-9a-f]+$/);

  await send(page, 'FRIEND-WORDS roll for me');
  await expect.poll(() => rows(world.linggenHome, theirs)).toContain('FRIEND-WORDS');
  expect(rows(world.linggenHome, mine), "the phone wrote into the builder's day").not.toContain('FRIEND-WORDS');

  // The builder talks in their own day, on this Mac: the phone, pinned to
  // that day's id, is streamed none of it.
  await request.post(`${world.url}/api/chat`, {
    data: { session_id: mine, project_root: world.work, message: 'BUILDER-ONLY words', skill_name: 'dice' },
  });
  await expect.poll(() => rows(world.linggenHome, mine)).toContain('BUILDER-ONLY reply.');
  await page.waitForTimeout(500);
  await expect(page.getByText('BUILDER-ONLY')).toHaveCount(0);
});
