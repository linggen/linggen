// Talking to Yinyue by holding her avatar (ui/src/lib/holdToTalk.mts; design
// § 3): hold → "Listening…", let go → what was heard shows and she answers.
// The microphone is Chromium's fake device (no real one, no sound); her
// hearing is faked at the page's fetch (`/api/stt` is the engine's other
// half, built apart — the answer here is canned). Her voice stays muted (the
// fixture's default), so she answers in text and no clip is made or played.
import type { Page } from '@playwright/test';
import { test, expect, expectWebRtc } from '../support/world';

const HEARD = 'what is on today';

/** Her hearing, canned, at the one place the page asks it: `window.fetch`.
 *  The UI replaces `window.fetch` when it starts (the data-channel proxy) and
 *  keeps the browser's own for signaling, so this is an accessor: the UI's
 *  assignment lands in `routed`, and the calls the proxy would take go there,
 *  the rest to the browser's fetch, `/api/stt` to the canned answer. Also
 *  keeps every microphone stream the page opens, to read its tracks later. */
function fakeHearing(answer: { status: number; body: unknown }) {
  const w = window as unknown as {
    __stt: { body: string }[]; __chat: unknown[]; __streams: MediaStream[]; __sttAnswer: typeof answer;
  };
  w.__stt = [];
  w.__chat = [];
  w.__streams = [];
  w.__sttAnswer = answer;
  const native: typeof fetch = window.fetch.bind(window);
  let routed: typeof fetch | null = null;
  const rides = (p: string) => (p.startsWith('/api/') && !p.startsWith('/api/rtc/'))
    || p.startsWith('/assets/') || p.startsWith('/apps/') || p === '/logo.svg';
  const wrapper = (async (input: RequestInfo | URL, init?: RequestInit) => {
    const raw = typeof input === 'string' ? input : input instanceof URL ? input.href : input.url;
    const path = new URL(raw, location.href).pathname;
    if (path === '/api/stt') {
      w.__stt.push({ body: String(init?.body ?? '') });
      return new Response(JSON.stringify(w.__sttAnswer.body), { status: w.__sttAnswer.status, headers: { 'Content-Type': 'application/json' } });
    }
    if (path === '/api/yinyue/chat') w.__chat.push(JSON.parse(String(init?.body ?? '{}')));
    return routed && rides(path) ? routed(input, init) : native(input, init);
  }) as typeof fetch;
  Object.defineProperty(window, 'fetch', {
    configurable: true,
    get: () => wrapper,
    set: (next) => { routed = next; },
  });
  const ask = navigator.mediaDevices.getUserMedia.bind(navigator.mediaDevices);
  navigator.mediaDevices.getUserMedia = async (c) => {
    const stream = await ask(c);
    w.__streams.push(stream);
    return stream;
  };
}

const her = (page: Page) => page.getByTitle('Talk to Yinyue');
const liveTracks = (page: Page) => page.evaluate(() =>
  (window as unknown as { __streams: MediaStream[] }).__streams
    .flatMap((s) => s.getTracks()).filter((t) => t.readyState === 'live').length);

async function holdHer(page: Page) {
  const box = (await her(page).boundingBox())!;
  await page.mouse.move(box.x + box.width / 2, box.y + box.height / 2);
  await page.mouse.down();
}

test.describe('asked aloud by a held press', () => {
  test.use({
    throttleFrames: true,
    scenario: { scripts: { yinyue: [{ text: "I'm here." }, { text: 'Nothing on today. Enjoy the quiet.' }] } },
  });

  test('hold, speak, let go: the heard line shows, her turn runs on it, the microphone is off', async ({ page, context, world, request, httpRequests }) => {
    await context.grantPermissions(['microphone'], { origin: world.url });
    await page.addInitScript(fakeHearing, { status: 200, body: { text: HEARD, language: 'en', ms: 400 } });
    await page.goto(world.url);
    await expectWebRtc(page, httpRequests);
    // She meets the person first (her door answers 409 before that).
    await request.post(`${world.url}/api/yinyue/chat`, { data: { text: 'Hello' } });
    await expect.poll(async () => (await world.calls()).filter((c) => c.agent === 'yinyue').length).toBe(1);

    await holdHer(page);
    await expect(page.getByTestId('talk-state')).toHaveText('Listening…');
    expect(await liveTracks(page)).toBe(1);
    await page.waitForTimeout(800); // a moment of the fake microphone's tone
    await page.mouse.up();

    // What was heard is shown, then her answer; nothing opened the typing box.
    await expect(page.getByTestId('talk-heard')).toHaveText(`“${HEARD}”`);
    // Her turn runs on the heard words: asked aloud, once, through her door.
    await expect.poll(async () => (await world.calls()).filter((c) => c.agent === 'yinyue').length, { timeout: 30_000 }).toBe(2);
    const chats = await page.evaluate(() => (window as unknown as { __chat: unknown[] }).__chat);
    expect(chats).toEqual([{ text: HEARD, asked_aloud: true }]);
    // Her answer reaches the page: in her bubble or her day's thread.
    await expect(page.getByText('Nothing on today.').first()).toBeVisible({ timeout: 20_000 });
    expect(await liveTracks(page)).toBe(0);
    await expect(page.getByPlaceholder('Talk to Yinyue…')).toHaveCount(0);

    // One clip went to her hearing: a 16 kHz mono 16-bit WAV.
    const sent = await page.evaluate(() => (window as unknown as { __stt: { body: string }[] }).__stt);
    expect(sent).toHaveLength(1);
    const wav = Buffer.from(JSON.parse(sent[0].body).wav_b64, 'base64');
    expect(wav.toString('latin1', 0, 4)).toBe('RIFF');
    expect(wav.readUInt32LE(24)).toBe(16_000);
    expect(wav.readUInt16LE(22)).toBe(1);
    expect(wav.readUInt16LE(34)).toBe(16);
    expect(wav.length).toBeGreaterThan(44 + 16_000 * 2 * 0.5);

  });

  test('a tap still opens the typing box, and opens no microphone', async ({ page, context, world, httpRequests }) => {
    await context.grantPermissions(['microphone'], { origin: world.url });
    await page.addInitScript(fakeHearing, { status: 200, body: { text: HEARD } });
    await page.goto(world.url);
    await expectWebRtc(page, httpRequests);
    await her(page).click();
    await expect(page.getByPlaceholder('Talk to Yinyue…')).toBeVisible();
    expect(await page.evaluate(() => (window as unknown as { __streams: unknown[] }).__streams.length)).toBe(0);
  });
});

test.describe('nothing heard', () => {
  test.use({ throttleFrames: true, scenario: { scripts: { yinyue: [{ text: "I'm here." }] } } });

  test('one quiet line, no turn for her, the microphone off', async ({ page, context, world, request, httpRequests }) => {
    await context.grantPermissions(['microphone'], { origin: world.url });
    await page.addInitScript(fakeHearing, { status: 200, body: { text: '' } });
    await page.goto(world.url);
    await expectWebRtc(page, httpRequests);
    await request.post(`${world.url}/api/yinyue/chat`, { data: { text: 'Hello' } });
    await expect.poll(async () => (await world.calls()).filter((c) => c.agent === 'yinyue').length).toBe(1);

    await holdHer(page);
    await expect(page.getByTestId('talk-state')).toHaveText('Listening…');
    await page.waitForTimeout(800);
    await page.mouse.up();
    await expect(page.getByTestId('talk-state')).toHaveText("Didn't catch that.");
    expect(await liveTracks(page)).toBe(0);
    expect((await world.calls()).filter((c) => c.agent === 'yinyue')).toHaveLength(1);
  });
});
