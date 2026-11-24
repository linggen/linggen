// Settings → Notes: what Linggen remembers, by source. Linggen's own notes
// can be tended (the ⋯ menu, or a long press on the row); another agent's are
// shown with their file path and nothing in the page can change them.
// Every call rode the data channel (the page fixture fails any plain-HTTP
// data request). The notes are made-up fixtures in the world's own home.
import { mkdirSync, readFileSync, writeFileSync } from 'node:fs';
import path from 'node:path';
import type { Page } from '@playwright/test';
import { test, expect, expectWebRtc } from '../support/world';

const BUILD = [
  '---', 'topic: Build', 'scope: project:/work/app', '---', '',
  '## Use just test',
  '<!-- id: r000000001 · kind: rule · voice: agent · created: 2025-01-01 · updated: 2025-01-01 · source: codex -->',
  'Never cargo by hand.', '',
  '## Old wrong thing',
  '<!-- id: r000000002 · kind: gotcha · voice: agent · created: 2026-11-01 · updated: 2026-11-01 · source: codex · void: it was a test -->',
  '',
].join('\n');
const CLAUDE_NOTE = 'Use just test, from Claude Code.\n';

function seed(linggenHome: string) {
  const own = path.join(linggenHome, 'memory/projects/-work-app/build.md');
  mkdirSync(path.dirname(own), { recursive: true });
  writeFileSync(own, BUILD);
  const claude = path.join(path.dirname(linggenHome), '.claude/projects/-work-app/memory');
  mkdirSync(claude, { recursive: true });
  writeFileSync(path.join(claude, 'MEMORY.md'), '- [Build](build.md) — how to build\n');
  writeFileSync(path.join(claude, 'build.md'), CLAUDE_NOTE);
  return { own, claudeNote: path.join(claude, 'build.md') };
}

async function openNotes(page: Page, url: string, httpRequests: string[]) {
  await page.goto(`${url}/settings`);
  await expectWebRtc(page, httpRequests);
  await page.getByRole('button', { name: 'Notes', exact: true }).click();
  await expect(page.getByTestId('remembered-sources')).toBeVisible();
}

const note = (page: Page, id: string) => page.locator(`[data-testid="memory-note"][data-id="${id}"]`);

test('Notes sits in the Data group and lists Linggen and the agents it reads', async ({ page, world, httpRequests }) => {
  seed(world.linggenHome);
  await openNotes(page, world.url, httpRequests);
  await expect(page.locator('aside.md\\:flex')).toContainText('DataMemoryStorageNotes');
  const tabs = page.getByTestId('memory-source-tab');
  await expect(tabs).toHaveText([/Linggen/, /Claude Code/, /Codex/]);
});

test('the owner marks a note wrong from its menu, and the file says so', async ({ page, world, httpRequests }) => {
  const { own } = seed(world.linggenHome);
  await openNotes(page, world.url, httpRequests);
  await page.getByTestId('memory-topic').click();
  const row = note(page, 'r000000001');
  await expect(row).toContainText('Use just test');
  await expect(row).toHaveAttribute('data-void', 'false');

  await row.getByRole('button', { name: 'Options' }).click();
  await page.getByRole('menuitem', { name: 'Mark wrong' }).click();
  await page.getByRole('dialog').getByRole('textbox').fill('no longer true');
  await page.getByRole('dialog').getByRole('button', { name: 'OK' }).click();

  await expect(row).toHaveAttribute('data-void', 'true');
  await expect(row.getByText('Wrong', { exact: true })).toBeVisible();
  await expect.poll(() => readFileSync(own, 'utf8')).toContain('void: no longer true');

  // And the other way: the one already wrong is made right again.
  const old = note(page, 'r000000002');
  await old.getByRole('button', { name: 'Options' }).click();
  await page.getByRole('menuitem', { name: 'Mark right again' }).click();
  await expect(old).toHaveAttribute('data-void', 'false');
});

test('a long press opens the same menu, and delete asks first', async ({ page, world, httpRequests }) => {
  const { own } = seed(world.linggenHome);
  await openNotes(page, world.url, httpRequests);
  await page.getByTestId('memory-topic').click();
  const row = note(page, 'r000000002');
  const box = (await row.boundingBox())!;
  await page.mouse.move(box.x + 40, box.y + box.height / 2);
  await page.mouse.down();
  await page.waitForTimeout(700);
  await page.mouse.up();
  const menu = row.getByRole('menu');
  await expect(menu.getByRole('menuitem')).toHaveText(['Edit', 'Mark right again', 'Never bring up', 'Delete']);

  await menu.getByRole('menuitem', { name: 'Delete' }).click();
  await page.getByRole('alertdialog').getByRole('button', { name: 'Cancel' }).click();
  await expect(row).toBeVisible();
  await row.getByRole('button', { name: 'Options' }).click();
  await page.getByRole('menuitem', { name: 'Delete' }).click();
  await page.getByRole('alertdialog').getByRole('button', { name: 'OK' }).click();
  await expect(row).toHaveCount(0);
  await expect.poll(() => readFileSync(own, 'utf8')).not.toContain('Old wrong thing');
});

test('the owner edits a note and the review list shows the unread and the wrong', async ({ page, world, httpRequests }) => {
  const { own } = seed(world.linggenHome);
  await openNotes(page, world.url, httpRequests);
  const review = page.getByTestId('memory-review');
  await expect(review.getByTestId('memory-note')).toHaveCount(2);
  await expect(review).toContainText('Not read for 90 days');
  await expect(review).toContainText('Marked wrong: it was a test');

  await note(page, 'r000000001').getByRole('button', { name: 'Options' }).click();
  await page.getByRole('menuitem', { name: 'Edit' }).click();
  const dialog = page.getByRole('dialog', { name: 'Edit note' });
  await dialog.getByRole('textbox').first().fill('Use just test always');
  await dialog.getByRole('button', { name: 'Save' }).click();
  await expect.poll(() => readFileSync(own, 'utf8')).toContain('## Use just test always');
  // An edit counts as a change: the note is no longer unread for 90 days.
  await expect(review.getByTestId('memory-note')).toHaveCount(1);
});

test("another agent's notes are shown with their file, and nothing offers a change", async ({ page, world, httpRequests }) => {
  const { claudeNote } = seed(world.linggenHome);
  await openNotes(page, world.url, httpRequests);
  await page.getByTestId('memory-source-tab').filter({ hasText: 'Claude Code' }).click();
  const readOnly = page.getByTestId('memory-readonly');
  await expect(readOnly).toContainText('Read only');
  await readOnly.getByTestId('memory-topic').click();
  await readOnly.getByTestId('memory-file').filter({ hasText: 'build.md' }).click();
  await expect(page.getByTestId('memory-file-path')).toContainText('~/.claude/projects/-work-app/memory/build.md');
  await expect(page.getByTestId('memory-file-text')).toContainText('Use just test, from Claude Code.');
  await expect(readOnly.getByRole('button', { name: 'Options' })).toHaveCount(0);
  await expect(readOnly.getByRole('menu')).toHaveCount(0);
  expect(readFileSync(claudeNote, 'utf8')).toBe(CLAUDE_NOTE);
});
