// The chat's thinking-effort chip (doc/models.md § effort): it lists only what
// the session's model offers, a choice is the conversation's and survives a
// reload, and 自动 gives it back to the chat default.
import { test, expect, expectWebRtc } from '../support/world';

const FERRY = 'sess-1790000002-c0ac7ed2';
const REASONING = `
[[models]]
id = "reasoning"
provider = "openai"
url = "https://api.openai.com/v1"
model = "gpt-5.4"
api_key = "test-not-a-key"
`;

test.use({ viewport: { width: 1280, height: 1000 }, scenario: { configAppend: REASONING } });

test('the chip appears for a model that offers effort, keeps its choice over a reload, and hides for one that does not', async ({ page, world, httpRequests }) => {
  await page.goto(`${world.url}/?session=${FERRY}&lang=zh`);
  await expectWebRtc(page, httpRequests);
  const model = page.getByTestId('session-model');
  const chip = page.getByTestId('session-effort');

  // The standing model offers none: no chip.
  await expect(model).toBeVisible();
  await expect(chip).toHaveCount(0);

  await model.selectOption('reasoning');
  await expect(chip).toBeVisible();
  const labels = await chip.locator('option').allTextContents();
  expect(labels).toEqual(['思考: 自动', '思考: 低', '思考: 中', '思考: 高']);
  await expect(chip).toHaveValue('');

  await chip.selectOption('medium');
  await expect(chip).toHaveValue('medium');
  await page.reload();
  await expectWebRtc(page, httpRequests);
  await expect(page.getByTestId('session-model')).toHaveValue('reasoning');
  await expect(page.getByTestId('session-effort')).toHaveValue('medium');

  // A model without it hides the chip; the choice is kept for a model that has it.
  await page.getByTestId('session-model').selectOption('fake-ling');
  await expect(page.getByTestId('session-effort')).toHaveCount(0);
  await page.getByTestId('session-model').selectOption('reasoning');
  await expect(page.getByTestId('session-effort')).toHaveValue('medium');

  // 自动 gives it back to the chat default.
  await page.getByTestId('session-effort').selectOption('');
  await page.reload();
  await expectWebRtc(page, httpRequests);
  await expect(page.getByTestId('session-effort')).toHaveValue('');
});

const CLAUDE = `
[[models]]
id = "claude"
provider = "anthropic"
url = "https://api.anthropic.com"
model = "claude-sonnet-5-5"
api_key = "test-not-a-key"
`;

test.describe('the default model is a Claude model', () => {
  test.use({ scenario: { configAppend: CLAUDE, defaultModel: 'claude' } });

  test('the default card offers its five values, and the choice survives a reload', async ({ page, world, httpRequests }) => {
    await page.goto(`${world.url}/settings/models?lang=zh`);
    await expectWebRtc(page, httpRequests);
    const pick = page.getByTestId('default-effort');
    await expect(pick).toBeVisible();
    expect(await pick.locator('option').allTextContents()).toEqual(['自动', '低', '中', '高', '超高', '最高']);
    await pick.selectOption('xhigh');
    await expect(pick).toHaveValue('xhigh');
    await expect(pick).toBeEnabled();
    await page.reload();
    await expectWebRtc(page, httpRequests);
    await expect(page.getByTestId('default-effort')).toHaveValue('xhigh');
  });
});
