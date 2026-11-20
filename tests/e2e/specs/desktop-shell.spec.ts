// The desktop app's window shows the web UI itself: one shell. Inside the
// app (a stand-in gives the page the shell's IPC object, support/shell.ts)
// the header carries a tab bar — the chat home, then one tab per app, each
// app page kept running — and an app opens as a tab. In a plain browser
// nothing of that shows, and an app opens in a new browser tab as before.
//
// ONE_SHELL_SHOTS=<dir> also saves the screenshots the change was shown with.
import { mkdirSync, writeFileSync } from 'node:fs';
import type { Page } from '@playwright/test';
import { test, expect, expectWebRtc, watchAsked, asked } from '../support/world';
import { standInForShell, shellCalls } from '../support/shell';

const shots = process.env.ONE_SHELL_SHOTS;
const shot = async (page: Page, name: string) => {
  if (shots) await page.screenshot({ path: `${shots}/${name}.png` });
};

const tabBar = (page: Page) => page.getByRole('navigation', { name: 'Tabs' });
const tab = (page: Page, name: string) => tabBar(page).getByRole('button', { name, exact: true });
const diceFrame = (page: Page) => page.locator('iframe[title="dice"]');
const diceReady = (page: Page) => diceFrame(page).contentFrame().getByText('Harbor Table: Alex rolled 4');
const abacusFrame = (page: Page) => page.locator('iframe[title="Abacus"]');
const chatBox = (page: Page) => page.getByTestId('chat-input');
const back = (page: Page) => page.getByRole('button', { name: 'Back', exact: true });
/** When the frame's page started: the same number means it never reloaded. */
const loadedAt = (page: Page) => diceReady(page).evaluate(() => performance.timeOrigin);

function installApp(home: string) {
  const dir = `${home}/skills/late-app`;
  mkdirSync(dir, { recursive: true });
  writeFileSync(`${dir}/SKILL.md`, '---\nname: late-app\ndescription: A fixture installed during a test.\napp:\n  launcher: web\n  entry: index.html\n  title: Late App\n---\n\nA test app.\n');
  writeFileSync(`${dir}/index.html`, '<h1>Late App</h1>');
}

test.describe('inside the desktop app', () => {
  test.beforeEach(async ({ page }) => {
    await standInForShell(page);
  });

  test('opens on the chat home, with a tab per listed app, from the URL an old shell opens', async ({ page, world }) => {
    const errors: string[] = [];
    page.on('pageerror', (e) => errors.push(String(e)));
    await page.addInitScript(watchAsked);
    await page.goto(`${world.url}/?launcher=1`);
    await expect(tabBar(page).getByRole('button')).toHaveText(['Chat', 'Abacus', 'dice']);
    await expect(tab(page, 'Chat')).toHaveAttribute('aria-current', 'page');
    await expect(chatBox(page)).toBeVisible();
    await expect(page.getByText('Harbor table').first()).toBeVisible(); // the session list
    expect(await page.evaluate(() => Reflect.get(window, '__LINGGEN_VIEW__'))).toBe('main');
    // No app page is loaded until its tab is opened.
    await expect(page.locator('iframe[title="dice"], iframe[title="Abacus"]')).toHaveCount(0);
    // The presence beat the web UI sends now comes from the app's window too.
    await expect.poll(() => asked(page)).toContain('POST /api/presence');
    // Yinyue lives in the shell's own window: no avatar in the page. A browser
    // shows her within about five seconds (below); give her longer here.
    await page.waitForTimeout(8000);
    await expect(page.getByTitle('Talk to Yinyue')).toHaveCount(0);
    await shot(page, 'shell-1-chat-home');
    expect(errors).toEqual([]);
  });

  test('switching apps is one click and reloads nothing; the sky never covers an app', async ({ page, world }) => {
    await page.goto(`${world.url}/`);
    await expect(chatBox(page)).toBeVisible();
    const backdrop = () => page.evaluate(() => 'skyBackdrop' in document.documentElement.dataset);
    await expect.poll(backdrop).toBe(true);

    await tab(page, 'dice').click();
    await expect(diceReady(page)).toBeVisible();
    await expect(chatBox(page)).toBeHidden();
    await expect.poll(backdrop).toBe(false);
    const frame = diceFrame(page);
    await expect(frame).toHaveAttribute('src', '/apps/dice/index.html?app_mode=1&in_launcher=1');
    await expect(frame).toHaveAttribute('sandbox', 'allow-scripts allow-same-origin allow-popups allow-forms allow-downloads');
    const first = await loadedAt(page);
    await shot(page, 'shell-2-app-dice');

    await tab(page, 'Abacus').click();
    const abacus = abacusFrame(page).contentFrame();
    await abacus.getByRole('button', { name: 'Add one' }).click();
    await expect(abacus.getByText('Count: 1')).toBeVisible();
    await expect(diceFrame(page)).toBeHidden();
    await expect(diceReady(page)).toBeAttached();
    expect(await loadedAt(page)).toBe(first);
    await shot(page, 'shell-3-app-abacus-after-one-click');

    await tab(page, 'dice').click();
    await expect(diceReady(page)).toBeVisible();
    expect(await loadedAt(page)).toBe(first);
    await tab(page, 'Abacus').click();
    await expect(abacus.getByText('Count: 1')).toBeVisible();

    await tab(page, 'Chat').click();
    await expect(chatBox(page)).toBeVisible();
    await expect.poll(backdrop).toBe(true);
    await expect(abacus.getByText('Count: 1')).toBeAttached();
  });

  test('settings from the gear, and Back to the same tab; Cmd+R keeps the place', async ({ page, world }) => {
    await page.goto(`${world.url}/`);
    await tab(page, 'dice').click();
    await expect(diceReady(page)).toBeVisible();
    const first = await loadedAt(page);

    await page.getByTitle('Settings', { exact: true }).click();
    await expect(page).toHaveURL(`${world.url}/settings`);
    await expect(back(page)).toBeVisible();
    await expect(diceFrame(page)).toBeHidden();
    await expect(tabBar(page)).toBeHidden();
    await shot(page, 'shell-4-settings-from-gear');

    await back(page).click();
    await expect(diceReady(page)).toBeVisible();
    await expect(tab(page, 'dice')).toHaveAttribute('aria-current', 'page');
    expect(await loadedAt(page)).toBe(first);
    await shot(page, 'shell-5-back-on-the-same-tab');

    // Esc does what Back does.
    await page.getByTitle('Settings', { exact: true }).click();
    await expect(back(page)).toBeVisible();
    await page.keyboard.press('Escape');
    await expect(diceReady(page)).toBeVisible();
    expect(await loadedAt(page)).toBe(first);

    // Cmd+R on settings comes back to settings, still the app's window, and
    // Back lands on the tab last used.
    await page.getByTitle('Settings', { exact: true }).click();
    await page.reload();
    await expect(back(page)).toBeVisible();
    await back(page).click();
    await expect(diceReady(page)).toBeVisible();
    await expect(tab(page, 'dice')).toHaveAttribute('aria-current', 'page');
  });

  test("the shell's Settings… menu opens settings, with or without a section", async ({ page, world }) => {
    await page.goto(`${world.url}/`);
    await tab(page, 'dice').click();
    await expect(diceReady(page)).toBeVisible();
    // The native menu can open settings while input focus is inside the app.
    await diceReady(page).evaluate((el) => {
      (el as HTMLElement).tabIndex = -1;
      (el as HTMLElement).focus();
    });
    await page.evaluate(() => window.postMessage({ type: 'linggen:show-settings' }, '*'));
    await expect(back(page)).toBeVisible();
    await expect(page.locator('[tabindex="-1"]').filter({ has: back(page) })).toBeFocused();
    await expect(diceFrame(page)).toBeHidden();
    await page.keyboard.press('Escape');
    await expect(diceReady(page)).toBeVisible();

    await page.evaluate(() => window.postMessage({ type: 'linggen:show-settings', section: 'memory' }, '*'));
    await expect(page.getByRole('button', { name: 'Memory', exact: true })).toHaveAttribute('aria-current', 'true');
    await shot(page, 'shell-6-settings-from-the-menu-message');

    // A chat framed under an app opens Settings → Models in this window.
    await page.keyboard.press('Escape');
    await expect(diceReady(page)).toBeVisible();
    await diceFrame(page).contentFrame().locator('body').evaluate(() => Reflect.get(window.parent, '__linggenOpenSettings')('models'));
    await expect(page.getByRole('button', { name: 'Models', exact: true })).toHaveAttribute('aria-current', 'true');
  });

  test('opening settings without a section leaves the current section and history alone', async ({ page, world }) => {
    await page.goto(`${world.url}/`);
    await page.evaluate(() => window.postMessage({ type: 'linggen:show-settings', section: 'memory' }, location.origin));
    const memory = page.getByRole('button', { name: 'Memory', exact: true });
    await expect(memory).toHaveAttribute('aria-current', 'true');
    const before = await page.evaluate(() => history.length);
    // Wait for the message to reach the window before checking that it did nothing.
    await page.evaluate(() => new Promise<void>((resolve) => {
      window.addEventListener('message', function delivered(e) {
        if (e.data?.type !== 'linggen:show-settings') return;
        window.removeEventListener('message', delivered);
        requestAnimationFrame(() => resolve());
      });
      window.postMessage({ type: 'linggen:show-settings' }, location.origin);
    }));
    expect(await page.evaluate(() => history.length)).toBe(before);
    await expect(memory).toHaveAttribute('aria-current', 'true');
  });

  test('a skill card shown after reload can open its app without reloading the page', async ({ page, world, request }) => {
    await page.goto(`${world.url}/`);
    await expect(tab(page, 'dice')).toBeVisible();
    installApp(world.linggenHome);
    expect((await request.post(`${world.url}/api/skills/reload`, { data: {} })).ok()).toBe(true);
    await page.getByText('/late-app', { exact: true }).click();
    await expect(tab(page, 'Late App')).toHaveAttribute('aria-current', 'page');
    await expect(page.frameLocator('iframe[title="Late App"]').getByRole('heading', { name: 'Late App' })).toBeVisible();
  });

  test('the engine immediately stops hearing that chat home is in front when an app tab covers it', async ({ page, world }) => {
    await page.addInitScript(() => {
      const beats: unknown[] = [];
      Object.defineProperty(window, '__beats', { value: beats });
      const send = RTCDataChannel.prototype.send;
      RTCDataChannel.prototype.send = function (this: RTCDataChannel, data: string | Blob | ArrayBuffer | ArrayBufferView) {
        if (typeof data === 'string' && data.includes('"/api/presence"')) {
          beats.push(JSON.parse(data).body);
        }
        return send.call(this, data as string);
      } as typeof RTCDataChannel.prototype.send;
    });
    const beats = () => page.evaluate(() => Reflect.get(window, '__beats') as {
      surface: string; app?: string; focused: boolean; typing: boolean;
    }[]);
    await page.goto(`${world.url}/`);
    await expect.poll(async () => (await beats()).at(-1)).toMatchObject({ focused: true });
    const home = (await beats()).at(-1)!;
    expect(home.app).toBeUndefined();
    const before = (await beats()).length;
    await tab(page, 'dice').click();
    await expect.poll(async () => (await beats()).slice(before).at(-1), { timeout: 2000 })
      .toMatchObject({ surface: home.surface, focused: false, typing: false });
    await expect(diceReady(page)).toBeVisible();
    await tab(page, 'Chat').click();
    await expect.poll(async () => (await beats()).at(-1), { timeout: 2000 })
      .toMatchObject({ surface: home.surface, focused: true });
  });

  test('the tab last used opens next time; an app launch brings its tab and the window up', async ({ page, world }) => {
    await page.goto(`${world.url}/`);
    await tab(page, 'Abacus').click();
    await expect(abacusFrame(page)).toBeVisible();
    await page.reload();
    await expect(tab(page, 'Abacus')).toHaveAttribute('aria-current', 'page');
    await expect(abacusFrame(page).contentFrame().getByText('Count: 0')).toBeVisible();

    await tab(page, 'Chat').click();
    await expect(chatBox(page)).toBeVisible();
    const res = await fetch(`${world.url}/api/apps/launch`, {
      method: 'POST',
      headers: { 'Content-Type': 'application/json' },
      body: JSON.stringify({ skill: 'dice', view: 'table' }),
    });
    expect(res.ok).toBe(true);
    await expect(tab(page, 'dice')).toHaveAttribute('aria-current', 'page');
    await expect(diceFrame(page)).toHaveAttribute('src', '/apps/dice/index.html?app_mode=1&in_launcher=1&view=table');
    await expect.poll(() => shellCalls(page)).toEqual(['plugin:window|show', 'plugin:window|unminimize', 'plugin:window|set_focus']);
  });

  test('a skill card opens its app as a tab; a launch never opens an app the bar leaves out', async ({ page, world }) => {
    await page.goto(`${world.url}/`);
    await expect(chatBox(page)).toBeVisible();
    await page.getByText('/dice', { exact: true }).click();
    await expect(tab(page, 'dice')).toHaveAttribute('aria-current', 'page');
    await expect(diceReady(page)).toBeVisible();
    expect(page.context().pages()).toHaveLength(1);
    await expect.poll(() => shellCalls(page)).toEqual(['plugin:window|show', 'plugin:window|unminimize', 'plugin:window|set_focus']);

    // `list: false`: a launch leaves it out, the person's own click opens it.
    await tab(page, 'Chat').click();
    const res = await fetch(`${world.url}/api/apps/launch`, {
      method: 'POST',
      headers: { 'Content-Type': 'application/json' },
      body: JSON.stringify({ skill: 'fixture-options' }),
    });
    expect(res.ok).toBe(true);
    await page.waitForTimeout(1500);
    await expect(tab(page, 'Chat')).toHaveAttribute('aria-current', 'page');
    await expect(tab(page, 'Fixture Options')).toHaveCount(0);
    await page.getByText('/fixture-options', { exact: true }).click();
    await expect(tab(page, 'Fixture Options')).toHaveAttribute('aria-current', 'page');
    await expect(page.locator('iframe[title="Fixture Options"]').contentFrame().getByRole('heading', { name: 'Fixture Options' })).toBeVisible();
    expect(page.context().pages()).toHaveLength(1);
  });

  test("an app page that holds no body for Yinyue makes this window her bodyless stage, while it is in front", async ({ page, world }) => {
    await page.addInitScript(() => {
      const said: string[] = [];
      Object.defineProperty(window, '__yinyueSaid', { value: said });
      const send = RTCDataChannel.prototype.send;
      RTCDataChannel.prototype.send = function (this: RTCDataChannel, data: string | Blob | ArrayBuffer | ArrayBufferView) {
        if (typeof data === 'string' && data.includes('"yinyue_')) {
          const msg = JSON.parse(data);
          said.push(msg.type === 'yinyue_subscribe' ? `subscribe stage=${msg.stage}` : 'release');
        }
        return send.call(this, data as string);
      } as typeof RTCDataChannel.prototype.send;
    });
    const said = () => page.evaluate(() => [...(Reflect.get(window, '__yinyueSaid') as string[])]);
    await page.goto(`${world.url}/`);
    await tab(page, 'dice').click();
    await expect(diceReady(page)).toBeVisible();
    await page.waitForTimeout(1500);
    expect((await said()).filter((m) => m.startsWith('subscribe'))).toEqual([]);

    await diceReady(page).evaluate(() => window.parent.postMessage({ type: 'linggen:companion', companion: 'hidden' }, location.origin));
    await expect.poll(said).toContain('subscribe stage=true');
    // The chat home in front: the page's word no longer counts.
    await tab(page, 'Chat').click();
    await expect.poll(async () => (await said()).at(-1)).toBe('release');
    await tab(page, 'dice').click();
    await expect.poll(async () => (await said()).at(-1)).toBe('subscribe stage=true');
    // Settings over it: neither.
    await page.getByTitle('Settings', { exact: true }).click();
    await expect.poll(async () => (await said()).at(-1)).toBe('release');
  });
});

test('page Escape leaves unsaved text and editor focus in settings', async ({ page, world }) => {
  await page.goto(`${world.url}/settings`);
  await expect(back(page)).toBeVisible();
  // Exercise all editor shapes with real focus and key events.
  await back(page).evaluate((button) => {
    button.insertAdjacentHTML('afterend', `
      <input aria-label="Test field" value="unsaved">
      <select aria-label="Test select"><option>unsaved</option></select>
      <textarea aria-label="Test textarea">unsaved</textarea>
      <div contenteditable="true" aria-label="Test editor"><span>unsaved</span></div>
    `);
  });
  for (const name of ['Test field', 'Test select', 'Test textarea', 'Test editor']) {
    const field = page.locator(`[aria-label="${name}"]`);
    await field.press('Escape');
    await expect(page).toHaveURL(`${world.url}/settings`);
    await expect(field).toBeFocused();
    if (name === 'Test editor') await expect(field).toHaveText('unsaved');
    else await expect(field).toHaveValue('unsaved');
  }
  await back(page).press('Escape');
  await expect(page).toHaveURL(`${world.url}/`);
});

test.describe('a browser without a selected session', () => {
  test.use({ scenario: { noConfig: true } });

  test('a browser does not open an app launch that names no session', async ({ page, world, request, httpRequests }) => {
    installApp(world.linggenHome);
    expect((await request.post(`${world.url}/api/skills/reload`, { data: {} })).ok()).toBe(true);
    await page.addInitScript(() => {
      const seen = { launches: 0, opens: 0, omitSession: false };
      Object.defineProperty(window, '__seen', { value: seen });
      window.open = () => { seen.opens++; return null; };
      const prop = Object.getOwnPropertyDescriptor(RTCDataChannel.prototype, 'onmessage')!;
      Object.defineProperty(RTCDataChannel.prototype, 'onmessage', {
        ...prop,
        set(fn) {
          prop.set!.call(this, function (this: RTCDataChannel, e: MessageEvent) {
            const msg = typeof e.data === 'string' && JSON.parse(e.data);
            if (msg?.kind !== 'app_launched') return fn.call(this, e);
            // The current server uses a broadcast sentinel. Also exercise the
            // UI contract for an event with no session_id field at all.
            if (seen.omitSession) delete msg.session_id;
            fn.call(this, new MessageEvent('message', { data: JSON.stringify(msg) }));
            queueMicrotask(() => seen.launches++);
          });
        },
      });
    });
    await page.goto(world.url);
    await expectWebRtc(page, httpRequests);
    await expect(page.getByTestId('session-list-empty')).toBeVisible();
    await expect(page.getByText('/late-app', { exact: true })).toBeVisible();
    expect((await request.post(`${world.url}/api/apps/launch`, { data: { skill: 'late-app' } })).ok()).toBe(true);
    const seen = () => page.evaluate(() => Reflect.get(window, '__seen'));
    await expect.poll(seen).toMatchObject({ launches: 1, opens: 0 });
    await page.evaluate(() => { Reflect.get(window, '__seen').omitSession = true; });
    expect((await request.post(`${world.url}/api/apps/launch`, { data: { skill: 'late-app' } })).ok()).toBe(true);
    await expect.poll(seen).toMatchObject({ launches: 2 });
    expect(await seen()).toMatchObject({ opens: 0 });
  });
});

test('framed pages stay what they are even when every frame looks like the shell', async ({ page, world }) => {
  await standInForShell(page, { allFrames: true });
  await page.goto(`${world.url}/`);
  await tab(page, 'dice').click();
  await expect(diceReady(page)).toBeVisible();
  const embedded = diceFrame(page).contentFrame().frameLocator('iframe[title="Framed chat"]');
  await diceReady(page).evaluate(() => {
    const frame = document.createElement('iframe');
    frame.title = 'Framed chat';
    frame.src = '/embed';
    document.body.prepend(frame);
  });
  const input = embedded.locator('textarea').first();
  await expect(input).toBeVisible();
  expect(await input.evaluate(() => Reflect.get(window, '__LINGGEN_VIEW__'))).toBe('embed');
  await expect(embedded.getByRole('navigation', { name: 'Tabs' })).toHaveCount(0);
});

test.describe('in a plain browser', () => {
  for (const url of ['/', '/?launcher=1']) {
    test(`${url} is the web UI as before: no tab bar`, async ({ page, world }) => {
      await page.goto(`${world.url}${url}`);
      await expect(chatBox(page)).toBeVisible();
      await expect(page.getByText('Harbor table').first()).toBeVisible();
      await expect(page.getByTitle('Talk to Yinyue')).toBeVisible();
      await expect(tabBar(page)).toHaveCount(0);
      await expect(page.locator('iframe[title="dice"]')).toHaveCount(0);
      if (url === '/') await shot(page, 'browser-1-main');
    });
  }

  test('a skill card opens its app in a new browser tab', async ({ page, world }) => {
    await page.goto(`${world.url}/`);
    const card = page.getByText('/dice', { exact: true });
    await expect(card).toBeVisible();
    const [opened] = await Promise.all([page.context().waitForEvent('page'), card.click()]);
    await opened.waitForLoadState();
    expect(new URL(opened.url()).pathname).toBe('/apps/dice/index.html');
    await expect(opened.getByText('Harbor Table: Alex rolled 4')).toBeVisible();
    await expect(tabBar(page)).toHaveCount(0);
    if (shots) await opened.screenshot({ path: `${shots}/browser-2-skill-card-new-tab.png` });
  });

  test('at phone width it is the web UI as before', async ({ page, world }) => {
    await page.setViewportSize({ width: 390, height: 844 });
    await page.goto(`${world.url}/`);
    await expect(chatBox(page)).toBeVisible();
    await expect(page.getByText('Harbor Table. Alex rolled a 4.')).toBeVisible();
    await expect(tabBar(page)).toHaveCount(0);
    await shot(page, 'browser-3-phone');
  });
});

for (const face of ['browser', 'desktop app'] as const) {
  test(`${face}: Settings → Apps lists only the declared page`, async ({ page, world }) => {
    if (face === 'desktop app') await standInForShell(page);
    await page.goto(`${world.url}/`);
    await expect(chatBox(page)).toBeVisible();
    await page.getByTitle('Settings', { exact: true }).click();
    const apps = page.locator('aside > div').filter({ has: page.getByText('Apps', { exact: true }) });
    await expect(apps.getByRole('button')).toHaveText(['Fixture Options']);
    await apps.getByRole('button', { name: 'Fixture Options' }).click();
    const frame = page.locator('iframe[title="Fixture Options"]');
    await expect(frame).toHaveAttribute('src', '/apps/fixture-options/pages/options.html?embed=1');
    await expect(frame.contentFrame().getByRole('heading', { name: 'Fixture Options' })).toBeVisible();
  });
}
