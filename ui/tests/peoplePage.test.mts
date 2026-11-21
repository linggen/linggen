// Settings → People: where the page's pieces live and what the device list no
// longer does. The logic itself is tests/people.test.mts.
import assert from 'node:assert/strict';
import { readFileSync, readdirSync } from 'node:fs';
import { join, relative } from 'node:path';
import { test } from 'node:test';
import { fileURLToPath } from 'node:url';
import { SETTINGS_SECTIONS } from '../src/lib/settingsSections.mts';

const root = fileURLToPath(new URL('../src/', import.meta.url));
const files = readdirSync(root, { recursive: true, withFileTypes: true })
  .filter((entry) => entry.isFile() && /\.[cm]?[jt]sx?$/.test(entry.name))
  .map((entry) => relative(root, join(entry.parentPath, entry.name)));

test('no file of the page names the old device level route', () => {
  // A person's level is set by `PUT /api/people/{person}` alone; the device's
  // own route stays in the engine only as a pass-through.
  const named = files.filter((file) => /\/api\/pair\/devices\/[^'"`\s]*\/permission|pairApi\.setPermission/.test(readFileSync(join(root, file), 'utf8')));
  assert.deepEqual(named, []);
});

test('the device list sets no level', () => {
  const phone = readFileSync(join(root, 'components/PhoneTab.tsx'), 'utf8');
  assert.doesNotMatch(phone, /\bLEVELS\b|\bsetLevel\b|permission\??:/);
});

test('People is a declared section of Devices, drawn by PeopleTab', () => {
  const people = SETTINGS_SECTIONS.find((s) => s.id === 'people');
  assert.ok(people);
  assert.equal(people.group, 'Devices');
  assert.match(readFileSync(join(root, 'components/settings/sections.tsx'), 'utf8'), /PeopleTab/);
});

test('the People page reads and writes only through peopleApi', () => {
  const direct = files.filter((file) => file !== 'lib/endpoints.ts' && /['"`]\/api\/people/.test(readFileSync(join(root, file), 'utf8')));
  assert.deepEqual(direct, []);
});
