// What Yinyue tells of Ling's work, on a page that holds her
// (doc/yinyue-companion-spec.md § Herald): a run waits on the person while
// they are away; she says so, her bubble carries that question's own card,
// the page's card carries the folded "said while you were away" mark, and an
// answer from her bubble resolves the question on the page too.
import type { Page } from '@playwright/test';
import { test, expect, expectWebRtc, expectTurnOver, chat, send, shown } from '../support/world';

const FERRY = 'sess-1790000002-c0ac7ed2';
const write = (file: string) => ({ tool: 'Write', args: { path: file, content: 'hello' } });
const pageCard = (page: Page) => chat(page).getByTestId('permission-card');
const bubble = (page: Page) => page.getByTestId('yinyue-bubble');

test.describe('a wait told while the person is away', () => {
  test.use({
    scenario: {
      scripts: {
        ling: [
          { tool: 'AskUser', args: { questions: [{ question: 'Which box?', header: 'Box', options: [{ label: 'Red' }, { label: 'Blue' }] }] } },
          write('second.txt'),
          { text: 'Left both.' },
        ],
        yinyue: [{ text: "I'm here." }, { text: '<say>WAIT-LINE Ling needs your yes.</say>' }],
      },
    },
  });

  test('her bubble carries the waiting card, the page card the mark, and her answer resolves it', async ({ page, world, request, httpRequests }) => {
    // This page holds her but is not in front of the person.
    await page.addInitScript(() => { document.hasFocus = () => false; });
    await page.goto(`${world.url}/?session=${FERRY}`);
    await expectWebRtc(page, httpRequests);
    await request.post(`${world.url}/api/yinyue/chat`, { data: { text: 'Hello' } });
    await expect.poll(async () => (await world.calls()).filter((c) => c.agent === 'yinyue').length).toBe(1);
    await request.post(`${world.url}/api/yinyue/chat`, { data: { text: '/unmute' } });

    await send(page, 'Pick a box, then write.');
    const red = page.getByRole('button', { name: 'Red', exact: true });
    await expect(red).toBeVisible();
    // Away; the question is answered on the page, then the write waits.
    await request.post(`${world.url}/api/presence`, { data: { surface: 'user-turn', focused: false, typing: false, idle_ms: 600_000 } });
    await red.click();
    await expect(pageCard(page).getByText('Write second.txt', { exact: true })).toBeVisible();

    // She tells of it: her bubble shows her words and that question's card.
    await expect(bubble(page)).toContainText('WAIT-LINE Ling needs your yes.', { timeout: 30_000 });
    const inBubble = bubble(page).getByTestId('permission-card');
    await expect(inBubble.getByText('Write second.txt', { exact: true })).toBeVisible();
    // The page's card carries the folded mark of what she said.
    await expect(chat(page).getByTestId('told-aloud')).toContainText('said this while you were away');

    // Answered from her bubble: the page's card goes, the run goes on.
    await inBubble.getByRole('button', { name: /^Deny/ }).click();
    await expect(pageCard(page)).toHaveCount(0);
    await expect(shown(page, 'Left both.')).toHaveCount(1);
    await expectTurnOver(page);
    await expect(bubble(page).getByTestId('permission-card')).toHaveCount(0);
  });
});
