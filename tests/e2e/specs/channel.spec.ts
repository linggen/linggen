// The UI's data rides the data channel only — before it connects too. A call
// made before the channel is up waits for it (and every test already fails
// on any data call over plain HTTP); Yinyue's voice, bytes and all, comes
// over it as well.
import type { Page } from '@playwright/test';
import {
  test, expect, expectWebRtc, expectScriptedCalls, expectTurnOver, send, shown, watchAsked, asked,
} from '../support/world';

test.describe('before the channel connects', () => {
  test.use({ scenario: { scripts: { ling: [{ text: 'Hello from the fake model.' }] } } });

  test('a message sent before it is delivered once it is up', async ({ page, world, httpRequests }) => {
    // Hold signaling, so the page runs with no data channel until released.
    let release!: () => void;
    const signaled = new Promise<void>((resolve) => (release = resolve));
    await page.route('**/api/rtc/whip', async (route) => {
      await signaled;
      await route.continue();
    });
    await page.addInitScript(watchAsked);
    await page.goto(world.url);
    await expect(page.getByPlaceholder(/^Message/)).toBeVisible();
    await send(page, 'Say hello.');
    const connected = () => page.evaluate(() =>
      ((window as unknown as { __pcs?: RTCPeerConnection[] }).__pcs ?? []).some((pc) => pc.connectionState === 'connected'));
    expect(await connected()).toBe(false);
    expect(await asked(page)).toEqual([]); // nothing went: there is no channel yet

    release();
    await expectWebRtc(page, httpRequests);
    await expect(shown(page, 'Hello from the fake model.')).toHaveCount(1);
    await expectTurnOver(page);
    await expect(shown(page, 'Say hello.')).toHaveCount(1);
    await expect(shown(page, 'Hello from the fake model.')).toHaveCount(1);
    // The boot's own reads waited too, and went over the channel.
    expect(await asked(page)).toContain('GET /api/status');
    await expectScriptedCalls(world, ['ling']);
  });
});

/** Records each clip the page decodes for Yinyue's voice: its length. */
function watchVoice() {
  const clips: number[] = [];
  (window as unknown as { __voice: number[] }).__voice = clips;
  const decode = BaseAudioContext.prototype.decodeAudioData;
  BaseAudioContext.prototype.decodeAudioData = async function (this: BaseAudioContext, data: ArrayBuffer) {
    const buf = await decode.call(this, data);
    clips.push(buf.duration);
    return buf;
  } as typeof BaseAudioContext.prototype.decodeAudioData;
}

/** Click Yinyue's avatar and say `text` to her. */
async function talkToYinyue(page: Page, text: string) {
  await page.getByTitle('Talk to Yinyue').click();
  const box = page.getByPlaceholder('Talk to Yinyue…');
  await box.fill(text);
  await box.press('Enter');
}

test.describe("Yinyue's voice", () => {
  test.use({ scenario: { scripts: { yinyue: [{ text: 'Hello, Alex.' }] } }, throttleFrames: true });

  test('her words and her speech come over the channel', async ({ page, world, httpRequests }) => {
    await page.addInitScript(watchAsked);
    await page.addInitScript(watchVoice);
    await page.goto(world.url);
    await expectWebRtc(page, httpRequests);
    // The fixture home keeps her muted: unmute her, then talk.
    await talkToYinyue(page, '/unmute');
    await expect.poll(() => asked(page)).toContain('POST /api/yinyue/chat');
    await talkToYinyue(page, 'Hello?');

    await expect(page.getByText('Hello, Alex.')).toBeVisible();
    await expect.poll(() => asked(page)).toContain('POST /api/tts');
    // The whole clip arrived intact: the page decoded the world's 1.5 s
    // stand-in voice (tests/system/support/voice.rs) from the channel's bytes.
    await expect
      .poll(() => page.evaluate(() =>
        (window as unknown as { __voice: number[] }).__voice.map((s) => Math.round(s * 100) / 100)))
      .toEqual([1.5]);
    await expectScriptedCalls(world, ['yinyue']);
  });
});
