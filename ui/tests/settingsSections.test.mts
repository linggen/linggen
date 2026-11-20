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
  const renderers = objectEntries(mapping, 'sections');
  assert.deepEqual(new Set(renderers.keys()), ids);
  for (const { id, needsConfig } of SETTINGS_SECTIONS) {
    const render = renderers.get(id)!;
    assert.equal(needsConfig, ts.isCallExpression(render) && render.expression.getText() === 'configSection', `${id}: config wrapper and declaration disagree`);
  }
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
  assert.deepEqual(SETTINGS_SECTIONS.filter((s) => s.group === null).map((s) => s.id), ['general', 'account', 'desktop', 'agents', 'models', 'session']);
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

function isId(node: ts.Node | undefined): boolean {
  return ids.has(literal(node) ?? '');
}

function isSectionList(node: ts.ArrayLiteralExpression, file: string) {
  // Short lists of common words need settings context. Registry-shaped entries
  // (an id field), or a longer run of ids, need no naming convention.
  const settingsContext = /settings|section/i.test(file) || /settings|section|tab/i.test(node.parent.getText().split(/[=\x5b(]/u, 1)[0]);
  const structured = node.elements.filter((entry) => ts.isObjectLiteralExpression(entry) && entry.properties.some((prop) =>
    ts.isPropertyAssignment(prop) && (literal(prop.name) ?? prop.name.getText()) === 'id' && isId(prop.initializer)));
  return structured.length >= 2 || node.elements.filter(containsId).length >= (settingsContext ? 2 : 3);
}

function duplicate(node: ts.Node, file: string): boolean {
  if (ts.isArrayLiteralExpression(node)) return isSectionList(node, file);
  if (ts.isUnionTypeNode(node)) return node.types.filter(isId).length >= 2;
  if (ts.isObjectLiteralExpression(node)) {
    return node.properties.filter((prop) => ts.isPropertyAssignment(prop) && ids.has(literal(prop.name) ?? prop.name.getText()) && (
      !ts.isArrayLiteralExpression(prop.initializer) &&
      !(ts.isPropertyAccessExpression(prop.initializer) && prop.initializer.name.text === prop.name.getText())
    )).length >= 2;
  }
  if (ts.isTypeReferenceNode(node) && /^(Record|Set|ReadonlySet)$/.test(node.typeName.getText())) {
    return /\b(SettingsSectionId|SettingsTab)\b/.test(node.typeArguments?.[0]?.getText() ?? '') || isId(node.typeArguments?.[0]);
  }
  if (ts.isCaseClause(node)) return sectionComparison(node.parent.parent.expression, node.expression);
  return ts.isBinaryExpression(node) && /^[!=]==?$/.test(node.operatorToken.getText()) && (
    sectionComparison(node.left, node.right) || sectionComparison(node.right, node.left)
  );
}

function sectionComparison(key: ts.Expression, value: ts.Expression) {
  if (!isId(value)) return false;
  // These are established wire values, not settings dispatch.
  const field = ts.isPropertyAccessExpression(key) ? key.name.text : key.getText();
  return !((field === 'creator' && literal(value) === 'mission') ||
    (field === 'type' && literal(value) === 'tools') ||
    (field === 'from' && literal(value) === 'memory'));
}

const desktop = 'components/settings/DesktopSection.tsx';
const flagFile = /\b(?:start-at-login|pet-disabled|pet-always-on)\b/;

function inspect(tree: ts.SourceFile, file: string) {
  const violations: string[] = [];
  if (file !== declaration && /\b(?:HAS_SETTINGS|KNOWN_SECTIONS|sidebarZh|ManagementTab)\b/.test(tree.getFullText())) violations.push(`${file}: legacy section list`);
  walk(tree, (node) => {
    const line = tree.getLineAndCharacterOfPosition(node.getStart()).line + 1;
    if (file !== desktop && (ts.isStringLiteralLike(node) || ts.isTemplateLiteralToken(node)) && flagFile.test(node.text)) {
      violations.push(`${file}:${line}: flag file belongs in DesktopSection`);
    }
    if (file !== declaration && file !== mapping && duplicate(node, file)) {
      violations.push(`${file}:${line}: second settings list or dispatch`);
    }
  });
  return violations;
}

test('no second section list or flag writer anywhere in UI source', () => {
  const violations: string[] = [];
  for (const entry of readdirSync(root, { recursive: true, withFileTypes: true })) {
    if (!entry.isFile() || !/\.[cm]?[jt]sx?$/.test(entry.name)) continue;
    const file = relative(root, join(entry.parentPath, entry.name));
    violations.push(...inspect(source(file), file));
  }
  assert.deepEqual(violations, []);
});

const fixture = (code: string, file = 'lib/example.ts') => inspect(ts.createSourceFile(file, code, ts.ScriptTarget.Latest, true), file);

for (const [name, code] of [
  ['switch', "switch (key) { case 'memory': render(); }"],
  ['section comparison', "section === 'memory' ? Memory : Storage"],
  ['property comparison', "active.id === 'memory' ? Memory : Storage"],
  ['key comparison', "key === 'memory' ? Memory : key === 'storage' ? Storage : null"],
  ['identifier keys', 'const render = { memory: Memory, storage: Storage };'],
  ['settings list', "const sections = ['session', 'general'];"],
]) {
  test(`the lock rejects a duplicate ${name}`, () => {
    assert.throws(() => assert.deepEqual(fixture(code), []), /second settings list or dispatch/);
  });
}

test('the lock rejects all three flag paths outside DesktopSection', () => {
  for (const flag of ['start-at-login', 'pet-disabled', 'pet-always-on']) {
    const code = `workspaceApi.bash('touch ~/.linggen/${flag}');`;
    assert.throws(() => assert.deepEqual(fixture(code), []), /belongs in DesktopSection/);
    assert.deepEqual(fixture(code, desktop), []);
  }
});

test('the lock allows a short unrelated array even if its words are section ids', () => {
  assert.deepEqual(fixture("const eventScopes = ['session', 'general'];"), []);
});
