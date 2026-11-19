// Settings → Missions: the editor shows and saves the mission picked in the
// list. Found by the whole-code review (2026-11-19): moving from one mission
// to another loaded the second's text but kept the first's id, so Save wrote
// the second's text into the first; and deleting the mission being edited
// from the list left the editor open on it.
import { readFileSync } from 'node:fs';
import path from 'node:path';
import type { Page } from '@playwright/test';
import { test, expect, expectWebRtc } from '../support/world';

/** The page, then Settings → Missions through their own buttons (a direct
 *  visit to /settings would be a plain-HTTP page load of a route). */
async function openMissions(page: Page, url: string, httpRequests: string[]) {
  await page.goto(url);
  await expectWebRtc(page, httpRequests);
  await page.getByRole('button', { name: 'Settings', exact: true }).click();
  await expect(page.getByText('Back')).toBeVisible();
  await page.getByRole('button', { name: 'Missions', exact: true }).first().click();
}

const navRow = (page: Page, name: string) =>
  page.locator('div.group').filter({ has: page.getByRole('button', { name, exact: false }) }).first();

test('saving a mission picked after another writes only the one picked', async ({ page, world, httpRequests }) => {
  const file = (id: string) => path.join(world.linggenHome, 'missions', id, 'mission.md');
  const read = (id: string) => { try { return readFileSync(file(id), 'utf8'); } catch { return ''; } };
  const before = readFileSync(file('table-check'), 'utf8');
  await openMissions(page, world.url, httpRequests);

  // A second mission of the person's own (a built-in's instructions are
  // not theirs to save).
  await page.getByRole('button', { name: 'New Mission' }).click();
  await page.getByRole('textbox').fill('ferry-check');
  await page.keyboard.press('Enter');
  await page.getByRole('button', { name: 'Create', exact: true }).click();
  await expect.poll(() => read('ferry-check')).toContain('name: ferry-check');

  await page.getByRole('button', { name: /^table-check/ }).click();
  await expect(page.getByText('~/.linggen/missions/table-check/mission.md')).toBeVisible();
  await expect(page.locator('.cm-content')).toContainText('Read the dice table');

  await page.getByRole('button', { name: /^ferry-check/ }).click();
  await expect(page.getByText('~/.linggen/missions/ferry-check/mission.md')).toBeVisible();
  await expect(page.locator('.cm-content')).not.toContainText('Read the dice table');

  await page.locator('.cm-content').click();
  await page.keyboard.press('ControlOrMeta+End');
  await page.keyboard.type('\nZEBRA-MARK');
  await page.getByRole('button', { name: 'Save', exact: true }).click();
  await expect(page.getByText('Unsaved')).toHaveCount(0);

  await expect.poll(() => read('ferry-check')).toContain('ZEBRA-MARK');
  expect(readFileSync(file('table-check'), 'utf8')).toBe(before);
});

test('deleting the mission being edited closes its editor', async ({ page, world, httpRequests }) => {
  await openMissions(page, world.url, httpRequests);
  await page.getByRole('button', { name: /^table-check/ }).click();
  await expect(page.getByText('~/.linggen/missions/table-check/mission.md')).toBeVisible();

  const row = navRow(page, 'table-check');
  await row.hover();
  await row.getByTitle('Delete', { exact: true }).click();
  await row.getByTitle('Confirm delete').click();

  await expect(page.getByRole('button', { name: /^table-check/ })).toHaveCount(0);
  await expect(page.getByText('~/.linggen/missions/table-check/mission.md')).toHaveCount(0);
});
