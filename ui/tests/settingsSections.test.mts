import assert from 'node:assert/strict';
import { readFileSync, readdirSync } from 'node:fs';
import { join, relative } from 'node:path';
import { test } from 'node:test';
import { fileURLToPath } from 'node:url';
import ts from 'typescript';
import { APPS_GROUP, BARE_SECTIONS, SETTINGS_SECTIONS, resolveSettingsTab, sectionAvailable } from '../src/lib/settingsSections.mts';
import { appSettingsEntries } from '../src/components/settings/appSettings.mts';
import type { SkillAppConfig, SkillInfoFull } from '../src/types.ts';

const root = fileURLToPath(new URL('../src/', import.meta.url));
const declaration = 'lib/settingsSections.mts';
const fixtureSkills = fileURLToPath(new URL('../../tests/fixtures/', import.meta.url));
const appNames = new Set(readdirSync(fixtureSkills, { recursive: true, withFileTypes: true })
  .filter((entry) => entry.name === 'SKILL.md' && entry.isFile())
  .flatMap((entry) => {
    const md = readFileSync(join(entry.parentPath, entry.name), 'utf8');
    const yaml = md.split(/^---\s*$/m)[1] ?? '';
    const name = yaml.match(/^name: (\S+)$/m)?.[1];
    return name && /^app:/m.test(yaml) ? [name] : [];
  }));
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

test('mcp aliases Tools; the legacy model id is absent', () => {
  assert.equal(resolveSettingsTab('mcp'), 'tools');
  assert.equal(ids.has('model'), false);
});

const webApp = (name: string, app: Partial<SkillAppConfig> = {}): SkillInfoFull => ({
  name, description: '', content: '', source: { type: 'Global' }, tool_defs: [],
  app: { launcher: 'web', entry: 'index.html', ...app },
});

test('Apps depends on the skill declaration, not the app name or the launcher tab list', () => {
  const name = `new-app-${process.pid}`;
  assert.deepEqual(appSettingsEntries([
    webApp(name, { settings: 'pages/my options.html', title: ' Options ', list: false }),
    webApp('no-declaration'),
    webApp('not-a-web-app', { launcher: 'bash', settings: 'options.html' }),
  ]), [{ id: `app-${name}`, group: APPS_GROUP, label: 'Options', src: `/apps/${name}/pages/my%20options.html?embed=1` }]);
});

test('app settings href keeps paths and URL syntax inside the app route', () => {
  for (const settings of ['../other/index.html', '/index.html', 'pages/./index.html', 'pages\\index.html']) {
    assert.deepEqual(appSettingsEntries([webApp('test', { settings })]), []);
  }
  for (const name of ['.', '..', 'next/other', 'next\\other']) {
    assert.deepEqual(appSettingsEntries([webApp(name, { settings: 'index.html' })]), []);
  }
  assert.equal(appSettingsEntries([webApp('test', { settings: 'pages/%2e%2e?#.html' })])[0].src,
    '/apps/test/pages/%252e%252e%3F%23.html?embed=1');
});

const panel = 'pages/Settings/SettingsPanel.tsx';
const frames = new Set(['pages/Settings/SettingsHome.tsx', 'pages/Settings/BareSection.tsx']);
// The desktop app's tab bar: it lists apps by name for its order, and draws
// no settings of its own (the face lock is tests/oneShell.test.mts).
const launcher = 'apps/AppTabs.tsx';
const moduleOptions = { moduleResolution: ts.ModuleResolutionKind.Bundler, allowJs: true };
const moduleCache = ts.createModuleResolutionCache(root, (file) => file);

function resolveModule(specifier: string, file: string) {
  const resolved = ts.resolveModuleName(specifier, join(root, file), moduleOptions, ts.sys, moduleCache).resolvedModule;
  return resolved && relative(root, resolved.resolvedFileName);
}

function importBindings(clause: ts.ImportClause | undefined) {
  const names = new Map<string, string>();
  if (clause?.name) names.set(clause.name.text, 'default');
  const bindings = clause?.namedBindings;
  if (bindings && ts.isNamedImports(bindings)) {
    for (const entry of bindings.elements) names.set(entry.name.text, (entry.propertyName ?? entry.name).text);
  }
  return names;
}

function mappedComponents() {
  const imports = new Map<string, { file: string; name: string }>();
  for (const node of source(mapping).statements) {
    if (!ts.isImportDeclaration(node) || !ts.isStringLiteralLike(node.moduleSpecifier)) continue;
    const file = resolveModule(node.moduleSpecifier.text, mapping);
    if (!file) continue;
    for (const [local, name] of importBindings(node.importClause)) imports.set(local, { file, name });
  }
  const components = new Map<string, Set<string>>();
  for (const [id, render] of objectEntries(mapping, 'sections')) {
    let found = false;
    walk(render, (node) => {
      if (!(ts.isJsxOpeningElement(node) || ts.isJsxSelfClosingElement(node)) || !ts.isIdentifier(node.tagName)) return;
      const imported = imports.get(node.tagName.text);
      if (!imported) return;
      found = true;
      const names = components.get(imported.file) ?? new Set<string>();
      names.add(imported.name);
      components.set(imported.file, names);
    });
    assert.ok(found, `${id}: expected an imported section body in the renderer`);
  }
  return components;
}

const components = mappedComponents();

function moduleUse(node: ts.Node) {
  if (ts.isImportDeclaration(node) && ts.isStringLiteralLike(node.moduleSpecifier)) {
    const clause = node.importClause;
    const bindings = clause?.namedBindings;
    const names = !clause || (bindings && ts.isNamespaceImport(bindings)) ? null : [...importBindings(clause).values()];
    return { specifier: node.moduleSpecifier.text, names };
  }
  if (ts.isExportDeclaration(node) && node.moduleSpecifier && ts.isStringLiteralLike(node.moduleSpecifier)) {
    const clause = node.exportClause;
    const names = clause && ts.isNamedExports(clause) ? clause.elements.map((entry) => (entry.propertyName ?? entry.name).text) : null;
    return { specifier: node.moduleSpecifier.text, names };
  }
  if (ts.isCallExpression(node) && (node.expression.kind === ts.SyntaxKind.ImportKeyword ||
    (ts.isIdentifier(node.expression) && node.expression.text === 'require'))) {
    const [argument] = node.arguments;
    if (argument && ts.isStringLiteralLike(argument)) return { specifier: argument.text, names: null };
  }
}

function importViolation(node: ts.Node, file: string) {
  const use = moduleUse(node);
  if (!use) return;
  const target = resolveModule(use.specifier, file);
  if (!target) return;
  const bodies = components.get(target);
  if (bodies && file !== mapping) {
    const used = use.names === null ? [...bodies] : use.names.filter((name) => bodies.has(name));
    if (used.length) return `${used.join(', ')} may only be imported by ${mapping}`;
  }
  // Named exception: SettingsPanel renders the mapping for SettingsHome.
  if (file === panel && target === mapping) return;
  if ((target === mapping || target === panel) && !frames.has(file)) {
    return `${target} may only be imported by settings frames`;
  }
}

function drawsSettings(file: string) {
  return file.startsWith('pages/Settings/') || file.startsWith('components/settings/') || frames.has(file) || file === launcher;
}

function isId(node: ts.Node): boolean {
  return ts.isStringLiteralLike(node) && ids.has(node.text);
}

function duplicateDispatch(node: ts.Node, file: string): boolean {
  if (ts.isObjectLiteralExpression(node)) {
    const owner = ts.isSatisfiesExpression(node.parent) ? node.parent.parent : node.parent;
    if (file === mapping && ts.isVariableDeclaration(owner) && owner.name.getText() === 'sections') return false;
    return node.properties.filter(({ name }) => name && (
      ts.isComputedPropertyName(name) ? isId(name.expression) :
        (ts.isIdentifier(name) || ts.isStringLiteralLike(name)) && ids.has(name.text)
    )).length >= 2;
  }
  if (ts.isCaseClause(node)) return isId(node.expression);
  return ts.isBinaryExpression(node) && /^[!=]==?$/.test(node.operatorToken.getText()) &&
    (isId(node.left) || isId(node.right));
}

const desktop = 'components/settings/DesktopSection.tsx';
const flagFile = /\b(?:start-at-login|pet-disabled|pet-always-on)\b/;

function inspect(tree: ts.SourceFile, file: string) {
  const violations: string[] = [];
  if (/\b(?:HAS_SETTINGS|appHasSettings|KNOWN_SECTIONS|sidebarZh|ManagementTab)\b/.test(tree.getFullText())) violations.push(`${file}: legacy section list`);
  const scoped = drawsSettings(file);
  walk(tree, (node) => {
    const line = () => tree.getLineAndCharacterOfPosition(node.getStart()).line + 1;
    const access = importViolation(node, file);
    if (access) violations.push(`${file}:${line()}: ${access}`);
    if (file !== desktop && (ts.isStringLiteralLike(node) || ts.isTemplateLiteralToken(node)) && flagFile.test(node.text)) {
      violations.push(`${file}:${line()}: flag file belongs in DesktopSection`);
    }
    if ((file === declaration || file.startsWith('pages/Settings/') || file.startsWith('components/settings/'))
      && ts.isStringLiteralLike(node) && appNames.has(node.text)) {
      violations.push(`${file}:${line()}: app names belong in skill declarations`);
    }
    if (scoped && duplicateDispatch(node, file)) {
      violations.push(`${file}:${line()}: second settings dispatch`);
    }
  });
  return violations;
}

test('settings source respects section import, dispatch, and flag ownership', () => {
  const violations: string[] = [];
  for (const entry of readdirSync(root, { recursive: true, withFileTypes: true })) {
    if (!entry.isFile() || !/\.[cm]?[jt]sx?$/.test(entry.name)) continue;
    const file = relative(root, join(entry.parentPath, entry.name));
    violations.push(...inspect(source(file), file));
  }
  assert.deepEqual(violations, []);
});

const fixture = (code: string, file = 'lib/example.ts') => inspect(ts.createSourceFile(file, code, ts.ScriptTarget.Latest, true), file);

test('app-name literals from fixture declarations are banned only in shared settings', () => {
  assert.ok(appNames.size > 0);
  for (const name of appNames) {
    const code = `const names = [${JSON.stringify(name)}];`;
    assert.match(fixture(code, panel).join('\n'), /app names belong in skill declarations/);
    assert.match(fixture(code, declaration).join('\n'), /app names belong in skill declarations/);
    assert.match(fixture(code, 'components/settings/appSettings.mts').join('\n'), /app names belong in skill declarations/);
    assert.deepEqual(fixture(code, launcher), []);
  }
});

const otherApp = 'apps/OtherSettings.tsx';

test('a new app cannot assemble its own settings from section bodies', () => {
  const code = `import { MemoryTab } from '../components/memory/MemoryTab';
import { ModelsTab as Models } from '../components/ModelsTab.js';
export const OtherSettings = ({ showMemory }) => showMemory ? <MemoryTab /> : <Models />;`;
  assert.deepEqual(fixture(code, otherApp), [
    `${otherApp}:1: MemoryTab may only be imported by ${mapping}`,
    `${otherApp}:2: ModelsTab may only be imported by ${mapping}`,
  ]);
});

test('barrels, namespace imports, and literal dynamic imports cannot expose section bodies', () => {
  for (const code of [
    "export * from '../components/memory/MemoryTab';",
    "import * as memory from '../components/memory/MemoryTab';",
    "const memory = import('../components/memory/MemoryTab');",
  ]) {
    assert.deepEqual(fixture(code, otherApp), [`${otherApp}:1: MemoryTab may only be imported by ${mapping}`]);
  }
});

test('other exports of a section module are not section bodies', () => {
  assert.deepEqual(fixture("import { MissionEditor } from '../components/MissionPage';", otherApp), []);
});

test('the mapping and panel stay behind the settings frames', () => {
  const code = `import { SettingsSectionBody } from '../components/settings/sections';
import { SettingsPanel } from '../pages/Settings/SettingsPanel';`;
  assert.deepEqual(fixture(code, otherApp), [
    `${otherApp}:1: ${mapping} may only be imported by settings frames`,
    `${otherApp}:2: ${panel} may only be imported by settings frames`,
  ]);
  assert.deepEqual(fixture(code, launcher), fixture(code, otherApp).map((line) => line.replace(otherApp, launcher)));
  assert.deepEqual(fixture("import { SettingsPanel } from './SettingsPanel';", 'pages/Settings/SettingsHome.tsx'), []);
});

for (const [name, file, code] of [
  ['switch', launcher, "switch (key) { case 'memory': render(); }"],
  ['comparison chain', panel, "tab === 'memory' ? Memory : tab === 'models' ? Models : null"],
  ['property comparison', 'pages/Settings/SettingsHome.tsx', "active.id === 'memory' ? Memory : Storage"],
  ['id-keyed map', 'components/settings/ExtraPanel.tsx', "const render = { ['memory']: Memory, storage: Storage };"],
]) {
  test(`the lock rejects a duplicate ${name} in settings`, () => {
    assert.match(fixture(code, file).join('\n'), /second settings dispatch/);
  });
}

test('the lock rejects all three flag paths outside DesktopSection', () => {
  for (const flag of ['start-at-login', 'pet-disabled', 'pet-always-on']) {
    const code = `workspaceApi.bash('touch ~/.linggen/${flag}');`;
    assert.throws(() => assert.deepEqual(fixture(code), []), /belongs in DesktopSection/);
    assert.deepEqual(fixture(code, desktop), []);
  }
});

for (const [name, code] of [
  ['scope', "scope === 'session'"],
  ['kind', "r.kind === 'memory'"],
  ['source', "a.source === 'account'"],
  ['role', "m.role === 'tools'"],
  ['union type', "type Tier = 'session' | 'memory' | 'core'"],
  ['object keys', 'const counts = { session: 1, memory: 2 }'],
  ['array', "['agents', 'skills', 'tools']"],
]) {
  test(`the lock allows unrelated ${name} using section words`, () => {
    assert.deepEqual(fixture(code, 'lib/unrelatedSettingsMetrics.ts'), []);
  });
}
