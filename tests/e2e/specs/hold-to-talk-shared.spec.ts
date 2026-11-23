// An app's page loads the one hold-to-talk file the engine serves
// (/shared/hold-to-talk.js, the same file the main UI bundles): it arrives as
// JavaScript, imports nothing, and gives the page the control and its recorder.
import { test, expect } from '../support/world';

test("an app's page can load /shared/hold-to-talk.js", async ({ page, world }) => {
  await page.goto(`${world.url}/apps/dice/index.html`);
  const served = await page.evaluate(async () => {
    const file = '/shared/hold-to-talk.js';
    const reply = await fetch(file);
    const mod = await import(file);
    return { type: reply.headers.get('content-type'), control: typeof mod.createHoldToTalk, recorder: typeof mod.newSpeechContext };
  });
  expect(served.type).toContain('javascript');
  expect(served.control).toBe('function');
  expect(served.recorder).toBe('function');
});
