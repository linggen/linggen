// A day is its own person's (doc/one-ling-spec.md § One session a day). A
// phone paired to this Mac and signed in to another account reaches it the
// way a phone on the LAN does: its channel opened with the phone's token,
// every request riding that channel. It is given its own person's day, a
// message it sends naming the builder's day is refused, and nothing of the
// builder's day streams to it.
import { mkdirSync, readFileSync, writeFileSync } from 'node:fs';
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

  // A message from a page pinned to the builder's day is refused; one with
  // no session lands in the phone's own day.
  await send(page, 'FRIEND-WORDS roll for me');
  const own = await page.evaluate(async (work) => {
    const r = await fetch('/api/chat', {
      method: 'POST',
      headers: { 'content-type': 'application/json' },
      body: JSON.stringify({ project_root: work, message: 'FRIEND-OWN roll', skill_name: 'dice' }),
    });
    return (await r.json()).session_id;
  }, world.work);
  expect(own).toBe(theirs);
  await expect.poll(() => rows(world.linggenHome, theirs)).toContain('FRIEND-OWN');
  expect(rows(world.linggenHome, mine), "the phone wrote into the builder's day").not.toContain('FRIEND-');

  // The builder talks in their own day, on this Mac: the phone, pinned to
  // that day's id, is streamed none of it.
  await request.post(`${world.url}/api/chat`, {
    data: { session_id: mine, project_root: world.work, message: 'BUILDER-ONLY words', skill_name: 'dice' },
  });
  await expect.poll(() => rows(world.linggenHome, mine)).toContain('BUILDER-ONLY reply.');
  await page.waitForTimeout(500);
  await expect(page.getByText('BUILDER-ONLY')).toHaveCount(0);
});

/** Records every page_state the engine pushes on the page's channels. */
function recordPageStates() {
  const seen: unknown[] = [];
  (window as unknown as { __pageStates: unknown[] }).__pageStates = seen;
  const desc = Object.getOwnPropertyDescriptor(RTCDataChannel.prototype, 'onmessage')!;
  Object.defineProperty(RTCDataChannel.prototype, 'onmessage', {
    configurable: true,
    get() {
      return desc.get!.call(this);
    },
    set(fn: ((e: MessageEvent) => unknown) | null) {
      desc.set!.call(this, function (this: RTCDataChannel, e: MessageEvent) {
        try {
          const msg = JSON.parse(e.data);
          if (msg.kind === 'page_state') seen.push(msg.data);
        } catch {
          // not JSON: not a page state
        }
        return fn?.call(this, e);
      });
    },
  });
}

test.describe('page state and notes', () => {
  test.use({
    scenario: {
      scripts: {
        ling: [
          {
            tool: 'AskUser',
            args: { questions: [{ question: 'BUILDER-QUESTION which box?', header: 'Box', options: [{ label: 'Red' }, { label: 'Blue' }] }] },
          },
          { text: 'Red.' },
        ],
      },
    },
  });

  test("another person's phone, over the channel, is shown nothing of the builder's day and none of their notes", async ({ page, world, request, httpRequests }) => {
    pairFriendsPhone(world.linggenHome);
    await expect
      .poll(async () => (await request.get(`${world.url}/api/pair/me`, { headers: { 'x-linggen-device': PHONE.token } })).status())
      .toBe(200);
    const config = await (await request.get(`${world.url}/api/config`)).json();
    config.agent.one_session_a_day = true;
    await request.post(`${world.url}/api/config`, { data: config });
    const mine: string = (await (await request.get(`${world.url}/api/sessions/day`)).json()).id;
    mkdirSync(`${world.linggenHome}/notes`, { recursive: true });
    const note = { id: 'n1', text: 'BUILDER-NOTE the parcel', written_at: 0, device: 'mac' };
    writeFileSync(`${world.linggenHome}/notes/yinyue.json`, JSON.stringify({ next: 2, notes: [note] }));
    const own = await (await request.get(`${world.url}/api/notes?agent=yinyue`)).json();
    expect(own.notes, "the builder's own notes").toHaveLength(1);

    // The builder's day waits on a question.
    await request.post(`${world.url}/api/chat`, {
      data: { session_id: mine, project_root: world.work, message: 'Pick a box', skill_name: 'dice' },
    });
    await expect
      .poll(async () => JSON.stringify(await (await request.get(`${world.url}/api/pending-ask-user?skill=dice`)).json()))
      .toContain('BUILDER-QUESTION');

    // The friend's phone opens a page that says it views the builder's day.
    await page.addInitScript(recordPageStates);
    await page.route('**/api/rtc/whip', (route) =>
      route.continue({
        headers: { ...route.request().headers(), 'x-linggen-device': PHONE.token, 'x-linggen-actor-device': PHONE.id },
      }));
    await page.goto(`${world.url}/embed?skill=dice&session=${mine}&hide_toolbar=1`);
    await expectWebRtc(page, httpRequests);
    await expect(composer(page)).toBeVisible();
    await expect.poll(() => page.evaluate(() => (window as unknown as { __pageStates: unknown[] }).__pageStates.length)).toBeGreaterThan(0);
    await page.waitForTimeout(1000);

    const states = await page.evaluate(() => JSON.stringify((window as unknown as { __pageStates: unknown[] }).__pageStates));
    const notes = await page.evaluate(async () => (await (await fetch('/api/notes?agent=yinyue')).json()).notes);
    const wrong: string[] = [];
    if (states.includes(`"session_id":"${mine}"`)) wrong.push("a page state shows the builder's day");
    if (states.includes('BUILDER-QUESTION')) wrong.push("a page state shows the builder's question");
    if (JSON.stringify(notes).includes('BUILDER-NOTE')) wrong.push("the builder's notes were read");
    expect(wrong).toEqual([]);
  });
});

test.describe('every route', () => {
  test.use({
    scenario: {
      scripts: {
        ling: [
          {
            tool: 'AskUser',
            args: { questions: [{ question: 'BUILDER-QUESTION which box?', header: 'Box', options: [{ label: 'Red' }, { label: 'Blue' }] }] },
          },
          { text: 'Red.' },
        ],
      },
    },
  });

  test("another person's phone, over the channel, is refused the builder's day by every kind of route", async ({ page, world, request, httpRequests }) => {
    pairFriendsPhone(world.linggenHome);
    await expect
      .poll(async () => (await request.get(`${world.url}/api/pair/me`, { headers: { 'x-linggen-device': PHONE.token } })).status())
      .toBe(200);
    const config = await (await request.get(`${world.url}/api/config`)).json();
    config.agent.one_session_a_day = true;
    await request.post(`${world.url}/api/config`, { data: config });
    const mine: string = (await (await request.get(`${world.url}/api/sessions/day`)).json()).id;
    await request.post(`${world.url}/api/chat`, {
      data: { session_id: mine, project_root: world.work, message: 'BUILDER-WORDS pick a box', skill_name: 'dice' },
    });
    await expect
      .poll(async () => JSON.stringify(await (await request.get(`${world.url}/api/pending-ask-user?skill=dice`)).json()))
      .toContain('BUILDER-QUESTION');
    const asks = await (await request.get(`${world.url}/api/pending-ask-user?skill=dice`)).json();
    const qid: string = asks[0].question_id;

    await page.route('**/api/rtc/whip', (route) =>
      route.continue({
        headers: { ...route.request().headers(), 'x-linggen-device': PHONE.token, 'x-linggen-actor-device': PHONE.id },
      }));
    await page.goto(`${world.url}/embed?skill=dice&hide_toolbar=1`);
    await expectWebRtc(page, httpRequests);
    await expect(composer(page)).toBeVisible();

    const seen = await page.evaluate(
      async ({ mine, qid, work }) => {
        const call = async (method: string, url: string, body?: unknown) => {
          const init: RequestInit = { method, headers: { 'content-type': 'application/json' } };
          if (body !== undefined) init.body = JSON.stringify(body);
          try {
            const r = await fetch(url, init);
            return { status: r.status, text: await r.text() };
          } catch (e) {
            return { status: 0, text: String(e) };
          }
        };
        const q = encodeURIComponent(mine);
        return {
          read: await call('GET', '/api/pending-ask-user?skill=dice'),
          answer: await call('POST', '/api/ask-user-response', { question_id: qid, answers: [{ question_index: 0, selected: ['Red'], custom_text: null }] }),
          clear: await call('POST', '/api/chat/clear', { session_id: mine }),
          plan: await call('POST', '/api/plan/approve', { session_id: mine }),
          permission: await call('PATCH', '/api/sessions/permission', { session_id: mine, path: work, mode: 'admin' }),
          pictures: await call('GET', `/api/sessions/images?session_id=${q}&name=x.png`),
          prompt: await call('GET', `/api/chat/system-prompt?session_id=${q}&agent_id=ling&project_root=${encodeURIComponent(work)}`),
          her: await call('POST', '/api/yinyue/chat', { text: 'hi' }),
          remove: await call('DELETE', '/api/sessions/all', { session_id: mine }),
        };
      },
      { mine, qid, work: world.work },
    );
    const wrong: string[] = [];
    if (seen.read.text.includes('BUILDER-QUESTION')) wrong.push('read the question');
    for (const [what, r] of Object.entries(seen)) {
      // A question it may not answer looks like an unknown one (404).
      const refused = what === 'answer' ? [403, 404] : [403];
      if (what !== 'read' && !refused.includes(r.status)) wrong.push(`${what}: ${r.status} ${r.text.slice(0, 80)}`);
    }
    const still = JSON.stringify(await (await request.get(`${world.url}/api/pending-ask-user?skill=dice`)).json());
    if (!still.includes('BUILDER-QUESTION')) wrong.push('the question was answered');
    if (!rows(world.linggenHome, mine).includes('BUILDER-WORDS')) wrong.push('the day was cleared or deleted');
    expect(wrong).toEqual([]);
  });
});

/** Keeps the page's control channel (`window.__control`) and every line she
 *  is given to speak on it (`window.__heard`). */
function keepControlAndHer() {
  const w = window as unknown as { __control?: RTCDataChannel; __heard: string[] };
  w.__heard = [];
  const make = RTCPeerConnection.prototype.createDataChannel;
  RTCPeerConnection.prototype.createDataChannel = function (this: RTCPeerConnection, label: string, init?: RTCDataChannelInit) {
    const dc = make.call(this, label, init);
    if (label === 'control') {
      w.__control = dc;
      dc.addEventListener('message', (e: MessageEvent) => {
        try {
          const msg = JSON.parse(e.data);
          if (msg.kind === 'pet_speak') w.__heard.push(String(msg.text ?? ''));
        } catch {
          // not JSON: not her line
        }
      });
    }
    return dc;
  };
}

/** Opens the friend's phone's page on its own channel. */
async function openFriendsPage(page: import('@playwright/test').Page, url: string, httpRequests: string[]) {
  await page.addInitScript(keepControlAndHer);
  await page.route('**/api/rtc/whip', (route) =>
    route.continue({
      headers: { ...route.request().headers(), 'x-linggen-device': PHONE.token, 'x-linggen-actor-device': PHONE.id },
    }));
  await page.goto(`${url}/embed?skill=dice&hide_toolbar=1`);
  await expectWebRtc(page, httpRequests);
  await expect(composer(page)).toBeVisible();
}

/** Turns the setting on, with the friend's phone paired; the builder's day. */
async function dayOnWithFriend(world: { url: string; linggenHome: string }, request: import('@playwright/test').APIRequestContext) {
  pairFriendsPhone(world.linggenHome);
  await expect
    .poll(async () => (await request.get(`${world.url}/api/pair/me`, { headers: { 'x-linggen-device': PHONE.token } })).status())
    .toBe(200);
  const config = await (await request.get(`${world.url}/api/config`)).json();
  config.agent.one_session_a_day = true;
  await request.post(`${world.url}/api/config`, { data: config });
  return (await (await request.get(`${world.url}/api/sessions/day`)).json()).id as string;
}

test.describe('her stage', () => {
  test.use({ scenario: { scripts: { yinyue: [{ text: 'HER-LINE for the builder.' }] } } });

  test("another person's phone holding her stage is given none of the builder's words nor her lines to them", async ({ page, world, request, httpRequests }) => {
    const mine = await dayOnWithFriend(world, request);
    await openFriendsPage(page, world.url, httpRequests);
    const theirs: string = await page.evaluate(async () => (await (await fetch('/api/sessions/day')).json()).id);
    expect(theirs).not.toBe(mine);
    // The phone's app opens her stage, on its own day — as its stage page does.
    await page.evaluate((session) => {
      const w = window as unknown as { __control: RTCDataChannel };
      w.__control.send(JSON.stringify({ type: 'yinyue_subscribe', stage: true, session }));
    }, theirs);
    await page.waitForTimeout(500);

    // The builder talks to her on this Mac.
    await request.post(`${world.url}/api/yinyue/chat`, { data: { text: 'BUILDER-TO-HER hello' } });
    await expect.poll(async () => (await world.calls()).filter((c) => c.agent === 'yinyue').length).toBeGreaterThan(0);
    await page.waitForTimeout(1500);

    const heard: string[] = await page.evaluate(() => (window as unknown as { __heard: string[] }).__heard);
    const wrong: string[] = [];
    if (rows(world.linggenHome, theirs).includes('BUILDER-TO-HER')) wrong.push("the builder's words landed in the phone's day");
    if (heard.some((l) => l.includes('HER-LINE'))) wrong.push("the phone was given her line to the builder");
    expect(wrong).toEqual([]);
  });
});
