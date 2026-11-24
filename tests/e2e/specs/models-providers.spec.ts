// Settings → Models, the provider rows: paste a key once, tick a family, see
// what it runs and its dated estimated price, get one notice when it moved,
// go back, add a provider, add a model by name — every step on a scratch
// engine whose providers' model lists and day come from a fixture
// (`families` in the scenario → test-families.json: no provider is called).
// The key is a marker, never a real one.
import type { Locator, Page } from '@playwright/test';
import { test, expect, expectWebRtc } from '../support/world';

const KEY = 'test-only-marker';
const LISTS = { anthropic: ['claude-opus-5', 'claude-opus-5-5', 'claude-sonnet-5-5', 'claude-haiku-5-5'] };
/** A pinned Opus on the model before the family's current one. */
const PINNED_OPUS = `
[[models]]
id = "seed-opus"
provider = "anthropic"
url = "https://api.anthropic.com"
model = "claude-opus-5"
[models.family_binding]
family = "claude-opus"
preset = "anthropic"
mode = "pinned"
`;

test.use({ scenario: { families: { today: '2026-11-23', lists: LISTS } } });

const provider = (page: Page, name: string) => page.getByTestId('provider-row').filter({ hasText: name }).first();
const family = (row: Locator, label: string) => row.getByTestId('family-row').filter({ has: row.page().getByLabel(label, { exact: true }) });

async function openModels(page: Page, world: { url: string }, httpRequests: string[]) {
  await page.goto(`${world.url}/settings/models`);
  await expectWebRtc(page, httpRequests);
  await expect(page.getByTestId('models-page')).toBeVisible();
}

/** Open the Anthropic row and paste the key once. */
async function connectAnthropic(row: Locator) {
  await row.getByRole('button', { name: /Anthropic/ }).click();
  await row.getByTestId('key-input').fill(KEY);
  await row.getByRole('button', { name: 'Save key' }).click();
  await expect(row.getByTestId('key-set')).toBeVisible();
}

test.describe('a followed family', () => {
  test.use({ scenario: { families: { today: '2026-11-23', lists: LISTS }, configAppend: PINNED_OPUS } });

  test('Sonnet is ticked and unticked; a move shows one notice; Switch back, follow again, Got it; a reload keeps it', async ({ page, world, httpRequests }) => {
    await openModels(page, world, httpRequests);
    const row = provider(page, 'Anthropic');
    await connectAnthropic(row);

    // The key was pasted once and is never drawn again. The endpoint already
    // had a row, so it is not a new connection: no starter is ticked for it.
    await expect(page.getByTestId('models-page')).not.toContainText(KEY);
    const sonnet = family(row, 'Sonnet');
    await expect(sonnet).toContainText('Would be Claude Sonnet 5.5');
    await sonnet.getByRole('checkbox').click();
    await expect(sonnet.getByRole('checkbox')).toBeChecked();
    await expect(sonnet).toContainText('Now Claude Sonnet 5.5');
    await expect(sonnet).toContainText(/input \$[\d.]+ \/ output \$[\d.]+ per million/);
    await expect(sonnet).toContainText('estimate');
    await expect(row.getByTestId('price-date')).toContainText('2026-11-23');

    // Untick and tick again.
    await sonnet.getByRole('checkbox').click();
    await expect(sonnet.getByRole('checkbox')).not.toBeChecked();
    await sonnet.getByRole('checkbox').click();
    await expect(sonnet.getByRole('checkbox')).toBeChecked();

    // The seeded Opus is pinned on the older model; following it again moves it.
    const opus = family(row, 'Opus');
    await expect(opus).toContainText('Pinned');
    await expect(opus).toContainText(/Now Claude Opus 5(?!\.5)/);
    await opus.getByRole('button', { name: 'Options' }).click();
    await page.getByRole('menuitem', { name: 'Follow automatically again' }).click();
    const notice = opus.getByTestId('move-notice');
    await expect(notice).toContainText('Moved from claude-opus-5 to claude-opus-5-5');
    await expect(opus).toContainText('Now Claude Opus 5.5');

    // Switch back: the older model, pinned, the notice answered.
    await notice.getByRole('button', { name: 'Switch back' }).click();
    await expect(notice).toBeHidden();
    await expect(opus).toContainText(/Now Claude Opus 5(?!\.5)/);
    await expect(opus).toContainText('Pinned');

    // Follow again: the next notice; Got it answers it.
    await opus.getByRole('button', { name: 'Options' }).click();
    await page.getByRole('menuitem', { name: 'Follow automatically again' }).click();
    await expect(notice).toContainText('Moved from');
    await notice.getByRole('button', { name: 'Got it' }).click();
    await expect(notice).toBeHidden();

    // A reload keeps the ticks, the version and the answered notice.
    await page.reload();
    await expectWebRtc(page, httpRequests);
    const again = provider(page, 'Anthropic');
    await again.getByRole('button', { name: /Anthropic/ }).click();
    await expect(family(again, 'Sonnet').getByRole('checkbox')).toBeChecked();
    await expect(family(again, 'Opus').getByRole('checkbox')).toBeChecked();
    await expect(family(again, 'Opus')).toContainText('Now Claude Opus 5.5');
    await expect(again.getByTestId('move-notice')).toHaveCount(0);
  });
});

test('a provider is added from the list with one key; a model is added by name under Advanced', async ({ page, world, httpRequests }) => {
  await openModels(page, world, httpRequests);
  await page.getByTestId('add-provider-button').click();
  await page.getByTestId('add-provider').getByRole('button', { name: 'Anthropic', exact: true }).click();
  // The row opens at once with its one key field, and the pick is gone from the list.
  const row = provider(page, 'Anthropic');
  await expect(row.getByTestId('key-input')).toBeVisible();
  await row.getByTestId('key-input').fill(KEY);
  await row.getByRole('button', { name: 'Save key' }).click();
  await expect(family(row, 'Sonnet').getByRole('checkbox')).toBeChecked();
  await expect(family(row, 'Opus').getByRole('checkbox')).not.toBeChecked();
  await expect(family(row, 'Opus')).toContainText('Would be Claude Opus 5.5');

  await row.getByTestId('advanced').locator('summary').click();
  await row.getByLabel('Model name').fill('claude-typed-test');
  await row.getByRole('button', { name: 'Add', exact: true }).click();
  await expect(row.getByTestId('typed-model')).toContainText('claude-typed-test');
  await row.getByTestId('typed-model').getByRole('button', { name: 'Options' }).click();
  await page.getByRole('menuitem', { name: 'Remove' }).click();
  await expect(row.getByTestId('typed-model')).toHaveCount(0);
});

test('a default made on the page keeps the fallbacks, and a later edit elsewhere keeps the ticked rows', async ({ page, world, request, httpRequests }) => {
  const chain = async (): Promise<string[]> => (await (await request.get(`${world.url}/api/config`)).json()).routing.default_models;
  const before = await chain();
  expect(before.length, 'the fixture has a default').toBeGreaterThan(0);
  // Through the settings home: its tabs share one copy of the config, as a person's visit does.
  await page.goto(`${world.url}/`);
  await expectWebRtc(page, httpRequests);
  await page.getByTitle('Settings', { exact: true }).click();
  await page.getByRole('button', { name: 'Models', exact: true }).click();
  await expect(page.getByTestId('models-page')).toBeVisible();
  const row = provider(page, 'Anthropic');
  await page.getByTestId('add-provider-button').click();
  await page.getByTestId('add-provider').getByRole('button', { name: 'Anthropic', exact: true }).click();
  await row.getByTestId('key-input').fill(KEY);
  await row.getByRole('button', { name: 'Save key' }).click();
  await expect(family(row, 'Sonnet').getByRole('checkbox')).toBeChecked();
  await family(row, 'Haiku').getByRole('checkbox').click();
  await expect(family(row, 'Haiku').getByRole('checkbox')).toBeChecked();

  // Make Haiku the default from the card: it goes first, the old chain follows.
  await page.getByTestId('default-chat').selectOption({ label: 'Claude Haiku 5.5' });
  await expect(family(row, 'Haiku')).toContainText('Default chat');
  const after = await chain();
  expect(after[0]).not.toBe(before[0]);
  expect(after.slice(1)).toEqual(before);

  // An edit on another tab (a save of the whole config) does not remove what was ticked.
  await page.getByRole('button', { name: 'General', exact: true }).click();
  await page.getByText('Logging', { exact: true }).click();
  await page.locator('select').filter({ has: page.locator('option[value="trace"]') }).selectOption('debug');
  await expect.poll(async () => (await (await request.get(`${world.url}/api/config`)).json()).logging.level).toBe('debug');
  await page.getByRole('button', { name: 'Models', exact: true }).click();
  const again = provider(page, 'Anthropic');
  await again.getByRole('button', { name: /Anthropic/ }).click();
  await expect(family(again, 'Sonnet').getByRole('checkbox')).toBeChecked();
  await expect(family(again, 'Haiku').getByRole('checkbox')).toBeChecked();
  expect(await chain()).toEqual(after);
});

for (const [label, model] of [['Google Gemini', 'gemini-3.8-live'], ['Qwen Cloud', 'qwen-audio-3.1-realtime-flash']]) {
  test(`${label} shows its declared live model without a chat row`, async ({ page, world, httpRequests }) => {
    await page.goto(`${world.url}/settings/models?lang=zh`);
    await expectWebRtc(page, httpRequests);
    const row = provider(page, label);
    await row.getByRole('button').first().click();
    const line = row.getByTestId('live-model');
    await expect(line).toHaveCount(1);
    await expect(line).toContainText(model);
    await expect(line).toContainText('实时语音');
    await expect(line).toContainText('缺密钥');
    await row.getByTestId('key-input').fill(KEY);
    await row.getByTestId('key-input').press('Enter');
    await expect(line).toContainText('可用');
    await expect(line).toContainText('先用');
    await expect(line.getByRole('checkbox')).toHaveCount(0);
    await expect(page.getByTestId('default-chat').locator('option').filter({hasText: model})).toHaveCount(0);
  });
}

test.describe('a narrow window', () => {
  test.use({ viewport: { width: 420, height: 900 } });

  test('rows stack and nothing scrolls sideways', async ({ page, world, httpRequests }) => {
    await openModels(page, world, httpRequests);
    await page.getByTestId('add-provider-button').click();
    await page.getByTestId('add-provider').getByRole('button', { name: 'Anthropic', exact: true }).click();
    const row = provider(page, 'Anthropic');
    await row.getByTestId('key-input').fill(KEY);
    await row.getByRole('button', { name: 'Save key' }).click();
    await expect(family(row, 'Sonnet').getByRole('checkbox')).toBeChecked();
    await row.getByTestId('advanced').locator('summary').click();
    // The page, and every box around it up to the window, is no wider than what it shows.
    const wider = await page.getByTestId('models-page').evaluate((el) => {
      const out: string[] = [];
      for (let n: Element | null = el; n; n = n.parentElement) {
        if (n.scrollWidth > n.clientWidth + 1) out.push(`${n.tagName}.${(n as HTMLElement).className.toString().slice(0, 40)} ${n.scrollWidth}>${n.clientWidth}`);
      }
      return out;
    });
    expect(wider).toEqual([]);
  });
});
