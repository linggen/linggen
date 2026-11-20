import assert from 'node:assert/strict';
import { readFileSync, readdirSync } from 'node:fs';
import { join, relative } from 'node:path';
import { test } from 'node:test';
import { fileURLToPath } from 'node:url';
import ts from 'typescript';
import { APPS_GROUP, BARE_SECTIONS, SETTINGS_SECTIONS, appHasSettings, resolveSettingsTab, sectionAvailable } from '../src/lib/settingsSections.mts';

const root = fileURLToPath(new URL('../src/', import.meta.url));
const declaration = 'lib/settingsSections.mts';
const mapping = 'components/settings/sections.tsx';
const ids = new Set<string>(SETTINGS_SECTIONS.map(({ id }) => id));
const source = (file: string) => ts.createSourceFile(file, readFileSync(join(root, file), 'utf8'), ts.ScriptTarget.Latest, true);

function walk(node: ts.Node, visit: (node: ts.Node) => void) {
  visit(node);
  ts.forEachChild(node, (child) => walk(child, visit));
}

function objectEntries(file: string, name: string) {
  const entries = new Map<string, ts.Expression>();
  walk(source(file), (node) => {
    if (!ts.isVariableDeclaration(node) || node.name.getText() !== name || !node.initializer) return;
    const value = ts.isSatisfiesExpression(node.initializer) ? node.initializer.expression : node.initializer;
    assert.ok(ts.isObjectLiteralExpression(value));
    for (const prop of value.properties) {
      assert.ok(ts.isPropertyAssignment(prop));
      const key = ts.isStringLiteral(prop.name) ? prop.name.text : prop.name.getText();
      assert.ok(!entries.has(key), `Duplicate key: ${key}`);
      entries.set(key, prop.initializer);
    }
  });
  return entries;
}

test('settings ids are unique and each has a renderer and Chinese label', () => {
  assert.equal(ids.size, SETTINGS_SECTIONS.length);
  assert.deepEqual(new Set(objectEntries(mapping, 'sections').keys()), ids);
  const labels = objectEntries('lib/strings-zh.ts', 'ZH');
  for (const label of [...SETTINGS_SECTIONS.flatMap((s) => s.group ? [s.label, s.group] : [s.label]), APPS_GROUP]) {
    const value = labels.get(label);
    assert.ok(value && ts.isStringLiteral(value) && /\p{Script=Han}/u.test(value.text), `No Chinese label for ${label}`);
  }
});

test('bare settings uses the declared subset and still gates Room', () => {
  const route = source('pages/Settings/BareSection.tsx').getFullText();
  assert.match(route, /BARE_SECTIONS\.find\(/);
  assert.deepEqual(BARE_SECTIONS, SETTINGS_SECTIONS.filter((s) => s.bare));
  assert.equal(BARE_SECTIONS.length, 7);
  const off = { rooms: false, companion: false, shell: false };
  const room = BARE_SECTIONS.find((s) => s.id === 'room')!;
  assert.equal(sectionAvailable(room, off), false);
  assert.equal(sectionAvailable(room, { ...off, rooms: true }), true);
  assert.equal(SETTINGS_SECTIONS.filter((s) => sectionAvailable(s, off)).length, 11);
  const desktop = SETTINGS_SECTIONS.find((s) => s.id === 'desktop')!;
  assert.equal(sectionAvailable(desktop, { ...off, companion: true }), false);
  assert.equal(sectionAvailable(desktop, { ...off, shell: true }), true);
});

test('mcp aliases Tools; the legacy model id and retired app entries are absent', () => {
  assert.equal(resolveSettingsTab('mcp'), 'tools');
  assert.equal(ids.has('model'), false);
  assert.equal(appHasSettings('pulse'), true);
  for (const name of ['mac-shifu', 'sys-doctor', 'health']) assert.equal(appHasSettings(name), false);
});

function literal(node: ts.Node | undefined): string | undefined {
  if (!node) return undefined;
  if (ts.isStringLiteralLike(node)) return node.text;
  if (ts.isLiteralTypeNode(node)) return literal(node.literal);
  if (ts.isPropertyAssignment(node)) return literal(node.initializer);
  return undefined;
}

function containsId(node: ts.Node): boolean {
  return ids.has(literal(node) ?? '') || ts.forEachChild(node, (child) => containsId(child) || undefined) === true;
}

function duplicate(node: ts.Node): boolean {
  if (ts.isArrayLiteralExpression(node)) return node.elements.filter(containsId).length >= 2;
  if (ts.isUnionTypeNode(node)) return node.types.filter((type) => ids.has(literal(type) ?? '')).length >= 2;
  if (ts.isObjectLiteralExpression(node)) {
    return node.properties.filter((prop) => ts.isPropertyAssignment(prop) && (
      ids.has(literal(prop.name) ?? '') ||
      (ts.isObjectLiteralExpression(prop.initializer) && prop.initializer.properties.some((field) =>
        field.name?.getText() === 'label') && ids.has(prop.name.getText()))
    )).length >= 2;
  }
  if (ts.isTypeReferenceNode(node) && /^(Record|Set|ReadonlySet)$/.test(node.typeName.getText())) {
    return /\b(SettingsSectionId|SettingsTab|ManagementTab)\b/.test(node.typeArguments?.[0]?.getText() ?? '') ||
      ids.has(literal(node.typeArguments?.[0]) ?? '');
  }
  if (!ts.isBinaryExpression(node) || !/^[!=]==?$/.test(node.operatorToken.getText())) return false;
  const comparesTab = (value: ts.Expression, key: ts.Expression) =>
    /^(?:\w+\.)?(?:\w*[Tt]ab|sectionId)$/.test(key.getText()) && ids.has(literal(value) ?? '');
  return comparesTab(node.right, node.left) || comparesTab(node.left, node.right);
}

test('no second section list or flag writer in the launcher', () => {
  const violations: string[] = [];
  for (const entry of readdirSync(root, { recursive: true, withFileTypes: true })) {
    if (!entry.isFile() || !/\.[cm]?[jt]sx?$/.test(entry.name)) continue;
    const file = relative(root, join(entry.parentPath, entry.name));
    const tree = source(file);
    if (file !== declaration && /\b(?:HAS_SETTINGS|KNOWN_SECTIONS|sidebarZh)\b/.test(tree.getFullText())) violations.push(`${file}: legacy section list`);
    if (file.startsWith('apps/') && /\b(?:start-at-login|pet-disabled|pet-always-on)\b|workspaceApi\s*\.\s*bash|['"]\/api\/bash['"]/.test(tree.getFullText())) {
      violations.push(`${file}: flag writer belongs in DesktopSection`);
    }
    if (file === declaration || file === mapping) continue;
    walk(tree, (node) => {
      if (duplicate(node)) violations.push(`${file}:${tree.getLineAndCharacterOfPosition(node.getStart()).line + 1}: second settings list or dispatch`);
    });
  }
  assert.deepEqual(violations, []);
});
