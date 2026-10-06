// A write outside the session's grant asks. Deny writes nothing and the run
// goes on to its reply; Allow once writes; switching the folder to edit
// writes, and the next write in it does not ask.
import { existsSync, readFileSync } from 'node:fs';
import path from 'node:path';
import type { Page } from '@playwright/test';
import { test, expect, expectWebRtc, expectScriptedCalls, expectTurnOver, chat, send, shown } from '../support/world';

const FERRY = 'sess-1790000002-c0ac7ed2';

const write = (file: string) => ({ tool: 'Write', args: { path: file, content: 'hello' } });

const card = (page: Page) => chat(page).getByTestId('permission-card');

async function askToWrite(page: Page, httpRequests: string[], url: string, file: string) {
  await page.goto(`${url}/?session=${FERRY}`);
  await expectWebRtc(page, httpRequests);
  await send(page, `Write ${file} saying hello.`);
  await expect(card(page)).toBeVisible();
  await expect(card(page).getByText(`Write ${file}`, { exact: true })).toBeVisible();
}

test.describe('Deny', () => {
  test.use({ scenario: { scripts: { ling: [write('probe.txt'), { text: 'Left it.' }] } } });

  test('a permission ask renders and Deny works', async ({ page, world, httpRequests }) => {
    await askToWrite(page, httpRequests, world.url, 'probe.txt');
    await card(page).getByRole('button', { name: /^Deny/ }).click();

    // The denied call is shown as such, then the run's reply.
    const call = chat(page).locator('[data-testid="tool-call"][data-tool="Write"]');
    await expect(call).toHaveAttribute('data-status', 'failed');
    await expect(shown(page, 'Left it.')).toHaveCount(1);
    await expectTurnOver(page);
    await expect(card(page)).toHaveCount(0);
    expect(existsSync(path.join(world.work, 'probe.txt'))).toBe(false);
    await expectScriptedCalls(world, ['ling', 'ling']);
  });
});

test.describe('Allow once', () => {
  test.use({ scenario: { scripts: { ling: [write('probe.txt'), { text: 'Wrote it.' }] } } });

  test('writes the file in the workspace', async ({ page, world, httpRequests }) => {
    await askToWrite(page, httpRequests, world.url, 'probe.txt');
    await card(page).getByRole('button', { name: /^Allow once/ }).click();

    const call = chat(page).locator('[data-testid="tool-call"][data-tool="Write"]');
    await expect(call).toHaveAttribute('data-status', 'done');
    await expect(shown(page, 'Wrote it.')).toHaveCount(1);
    await expectTurnOver(page);
    await expect(card(page)).toHaveCount(0);
    expect(readFileSync(path.join(world.work, 'probe.txt'), 'utf8')).toBe('hello');
    await expectScriptedCalls(world, ['ling', 'ling']);
  });
});

test.describe('always allow', () => {
  test.use({
    scenario: {
      scripts: { ling: [write('one.txt'), { text: 'Wrote one.' }, write('two.txt'), { text: 'Wrote two.' }] },
    },
  });

  test('switching the folder to edit: the next write does not ask', async ({ page, world, httpRequests }) => {
    await askToWrite(page, httpRequests, world.url, 'one.txt');
    await card(page).getByRole('button', { name: /^Switch this folder to edit/ }).click();
    await expect(shown(page, 'Wrote one.')).toHaveCount(1);
    await expectTurnOver(page);
    expect(readFileSync(path.join(world.work, 'one.txt'), 'utf8')).toBe('hello');

    await send(page, 'Now two.txt.');
    await expect(shown(page, 'Wrote two.')).toHaveCount(1);
    await expectTurnOver(page);
    await expect(card(page)).toHaveCount(0);
    expect(readFileSync(path.join(world.work, 'two.txt'), 'utf8')).toBe('hello');
    await expectScriptedCalls(world, ['ling', 'ling', 'ling', 'ling']);
  });
});
