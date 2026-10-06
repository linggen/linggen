// A skill page's engine calls ride a data channel, like the UI's: through the
// shared helpers (/shared/api.js) and its own fetch('/api/…') alike. Standing
// alone it opens its own (a hidden channel frame, one peer); framed by the UI
// it asks the frame above. Only its files and signaling use plain HTTP — and
// every test already fails on any data call over it (world.ts), the page's
// frames and the pages it opens included.
import type { Frame, Page } from '@playwright/test';
import { test, expect, watchAsked, asked } from '../support/world';

const DICE = '/apps/dice/index.html';

/** The hidden frame a standalone skill page carries its calls on. */
function channelFrames(page: Page): Frame[] {
  return page.frames().filter((f) => new URL(f.url()).searchParams.get('channel') === '1');
}

/** Yinyue's voice, fetched by the page itself: its type and its length. */
async function speak(page: Page): Promise<{ type: string | null; secs: number }> {
  return page.evaluate(async () => {
    const res = await fetch('/api/tts', {
      method: 'POST',
      headers: { 'Content-Type': 'application/json' },
      body: JSON.stringify({ text: 'Roll the die.' }),
    });
    const clip = await new OfflineAudioContext(1, 1, 22050).decodeAudioData(await res.arrayBuffer());
    return { type: res.headers.get('content-type'), secs: Math.round(clip.duration * 100) / 100 };
  });
}

test('a skill page standing alone carries its calls on its own channel', async ({ page, world }) => {
  await page.addInitScript(watchAsked);
  await page.goto(world.url + DICE);
  await expect(page.getByText('Harbor Table: Alex rolled 4')).toBeVisible(); // its own file, HTTP
  await expect(page.getByText(/^Models: \d+$/)).toBeVisible(); // /shared/api.js
  await expect(page.getByText(/^Chats: \d+$/)).toBeVisible(); // its own fetch('/api/…')

  const frames = channelFrames(page);
  expect(frames).toHaveLength(1);
  expect(await asked(frames[0])).toEqual(expect.arrayContaining(['GET /api/models', 'GET /api/skill-sessions']));

  // Bytes come whole: the world's 1.5 s stand-in voice decodes from the
  // channel's reply (tests/system/support/voice.rs).
  expect(await speak(page)).toEqual({ type: 'audio/wav', secs: 1.5 });
  expect(await asked(frames[0])).toContain('POST /api/tts');
});

test('a call a skill page makes before its channel is up waits for it', async ({ page, world }) => {
  // Hold signaling, so the page runs with no data channel until released.
  let release!: () => void;
  const signaled = new Promise<void>((resolve) => (release = resolve));
  await page.route('**/api/rtc/whip', async (route) => {
    await signaled;
    await route.continue();
  });
  await page.addInitScript(watchAsked);
  await page.goto(world.url + DICE);
  await expect(page.getByText('Harbor Table: Alex rolled 4')).toBeVisible();
  await expect.poll(() => channelFrames(page).length).toBe(1);
  expect(await asked(channelFrames(page)[0])).toEqual([]); // nothing went: no channel yet
  await expect(page.locator('#models')).toHaveText('…');

  release();
  await expect(page.getByText(/^Models: \d+$/)).toBeVisible();
  await expect(page.getByText(/^Chats: \d+$/)).toBeVisible();
});

test('a skill page framed by the launcher rides its channel — no peer of its own', async ({ page, world }) => {
  await page.addInitScript(watchAsked);
  await page.goto(`${world.url}/?launcher=1`);
  const dice = page.frameLocator('iframe[title="dice"]');
  await expect(dice.getByText(/^Models: \d+$/)).toBeVisible();
  await expect(dice.getByText(/^Chats: \d+$/)).toBeVisible();

  expect(channelFrames(page)).toEqual([]);
  // The launcher asked for it, over its own channel.
  await expect.poll(() => asked(page)).toContain('GET /api/skill-sessions');
});
