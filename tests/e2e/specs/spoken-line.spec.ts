// A line an app's page heard spoken (its microphone) and sent through the
// chat bridge: `chat.send(text, {heard: true})`. It goes the whole way —
// bridge, embed, request, saved row — and the chat draws it with the small
// mic mark, as it does again after a reload; a typed line has none.
import { test, expect } from '../support/world';

test.use({ scenario: { scripts: { ling: [{ text: 'First answered.' }, { text: 'Second answered.' }] } } });

test('a line the app page heard is kept and shown as spoken; a typed one is not', async ({ page, world, request }) => {
  const session = (await (await request.post(`${world.url}/api/sessions`, { data: { title: 'dice', skill: 'dice' } })).json()).id;
  await page.goto(`${world.url}/apps/dice/index.html`);
  await page.evaluate(async (sid) => {
    const bridge = '/shared/chat-bridge.js';
    const { mount } = await import(bridge);
    const el = document.createElement('div');
    el.style.height = '480px';
    document.body.appendChild(el);
    (window as unknown as { __chat: unknown }).__chat = await mount(el, { skillName: 'dice', agentId: 'ling', sessionId: sid });
  }, session);
  const chat = page.frameLocator('iframe[src*="/embed"]');
  const send = (line: string, opts: object) => page.evaluate(
    ([text, o]) => (window as unknown as { __chat: { send: (t: string, o: object) => void } }).__chat.send(text as string, o as object),
    [line, opts],
  );
  const mark = chat.getByTestId('heard-mark');

  await send('Bonjour, je suis Alex', { heard: true });
  await expect(chat.getByText('Bonjour, je suis Alex')).toHaveCount(1);
  await expect(chat.getByText('First answered.')).toHaveCount(1);
  await expect(mark).toHaveCount(1);

  await send('A typed line', {});
  await expect(chat.getByText('Second answered.')).toHaveCount(1);
  await expect(mark, 'the typed line carries no mark').toHaveCount(1);

  // The saved row says so: it is there again after a reload.
  await page.reload();
  await page.evaluate(async (sid) => {
    const bridge = '/shared/chat-bridge.js';
    const { mount } = await import(bridge);
    const el = document.createElement('div');
    el.style.height = '480px';
    document.body.appendChild(el);
    await mount(el, { skillName: 'dice', agentId: 'ling', sessionId: sid });
  }, session);
  await expect(chat.getByText('Bonjour, je suis Alex')).toHaveCount(1);
  await expect(chat.getByTestId('heard-mark')).toHaveCount(1);
});
