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
  // 自动 is the leftmost stop, before the model's three levels.
  await expect(chip.locator('option')).toHaveCount(4);
  await expect(chip).toHaveAttribute('data-choice', 'auto');
  await chip.selectOption('low');
  await expect(chip).toHaveAttribute('data-choice', 'low');
  await page.reload();
  await expectWebRtc(page, httpRequests);
  await expect(page.getByTestId('session-model')).toHaveValue('reasoning');
  await expect(chip).toHaveAttribute('data-choice', 'low');

  // Changing models drops the session override and uses the new model's saved effort.
  await model.selectOption('fake-ling');
  await expect(chip).toHaveCount(0);
  await model.selectOption('reasoning');
  await expect(chip).toHaveAttribute('data-choice', 'auto');
  await chip.selectOption('high');
  await expect(chip).toHaveAttribute('data-choice', 'high');
  await chip.selectOption('auto');
  await expect(chip).toHaveAttribute('data-choice', 'auto');
});

test('a ChatGPT sign-in row offers low, medium and high before catalog metadata arrives', async ({ page, world, httpRequests }) => {
  await page.goto(`${world.url}/?session=${FERRY}&lang=zh`);
  await expectWebRtc(page, httpRequests);
  await page.getByTestId('session-model').selectOption('fake-sol');
  const chip = page.getByTestId('session-effort');
  await expect(chip).toBeVisible();
  await expect(chip.locator('option')).toHaveCount(4);
  await chip.selectOption('high');
  await expect(chip).toHaveAttribute('data-choice', 'high');
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
    const slider = pick.getByRole('slider');
    await expect(slider).toHaveAttribute('max', '5');
    await slider.press('End');
    await expect(pick).toHaveAttribute('data-choice', 'max');
    await expect(slider).toBeEnabled();
    await slider.press('ArrowLeft');
    await expect(pick).toHaveAttribute('data-choice', 'xhigh');
    await page.reload();
    await expectWebRtc(page, httpRequests);
    await expect(pick).toHaveAttribute('data-choice', 'xhigh');
    const model = page.getByTestId('default-chat');
    await model.selectOption('fake-sol');
    await expect(pick).toHaveAttribute('data-choice', 'auto');
    await slider.press('End');
    await expect(pick).toHaveAttribute('data-choice', 'high');
    await expect(slider).toBeEnabled();
    await model.selectOption('claude');
    await expect(pick).toHaveAttribute('data-choice', 'xhigh');
  });
});
