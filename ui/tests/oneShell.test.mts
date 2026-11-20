// The lock for "one shell" (doc/code-style.md, principle 2): the desktop app
// and a browser render the same UI, and every difference between them is
// decided by one question, `inShell()` in lib/appHost.ts.
//
// It fails when:
//   - anything other than lib/appHost.ts reads `__TAURI_INTERNALS__`;
//   - the `launcher` query, a `launcher` view, or the old window mark is read;
//   - a second place decides where an app opens: a window is opened for an
//     app, the relay's app address is built, the tab host is registered, or
//     an app frame's URL is built outside the one file that owns it.
import assert from 'node:assert/strict';
import { readFileSync, readdirSync } from 'node:fs';
import { join, relative } from 'node:path';
import { test } from 'node:test';
import { fileURLToPath } from 'node:url';
import ts from 'typescript';

const root = fileURLToPath(new URL('../src/', import.meta.url));
const appHost = 'lib/appHost.ts';
const appTabs = 'apps/AppTabs.tsx';
/** Opens a window for something that is not an app, and says what. */
const OTHER_WINDOWS: ReadonlyMap<string, string> = new Map([
  ['components/chat/AnswerRefs.tsx', 'a link in an answer'],
  ['lib/ownModel.ts', 'the settings page, when no host can show it'],
  ['lib/endpoints.ts', 'the sign-in page, when the engine could not open it'],
  ['components/chat/AuthRequiredBlock.tsx', "a provider's sign-in page, when the engine could not open it"],
]);

function walk(node: ts.Node, visit: (node: ts.Node) => void) {
  visit(node);
  ts.forEachChild(node, (child) => walk(child, visit));
}

const textOf = (node: ts.Node) =>
  ts.isStringLiteralLike(node) || ts.isTemplateLiteralToken(node) ? node.text : null;

function opensWindow(node: ts.Node) {
  if (!ts.isCallExpression(node)) return false;
  const callee = node.expression;
  if (ts.isPropertyAccessExpression(callee)) {
    return callee.name.text === 'open' && /^(window|globalThis|self|top)$/.test(callee.expression.getText());
  }
  if (ts.isElementAccessExpression(callee)) {
    return textOf(callee.argumentExpression) === 'open';
  }
  return false;
}

function inspect(tree: ts.SourceFile, file: string) {
  const violations: string[] = [];
  walk(tree, (node) => {
    const at = () => `${file}:${tree.getLineAndCharacterOfPosition(node.getStart()).line + 1}`;
    const text = textOf(node);
    const name = ts.isIdentifier(node) || ts.isPrivateIdentifier(node) ? node.text : null;
    if (file !== appHost && (name === '__TAURI_INTERNALS__' || text?.includes('__TAURI_INTERNALS__'))) {
      violations.push(`${at()}: only ${appHost} asks whether this is the desktop shell`);
    }
    if (text !== null && (text === 'launcher' || /(?:^|[?&])launcher=/.test(text) || text.includes('launcher-face'))) {
      violations.push(`${at()}: the launcher face is gone; ask inShell()`);
    }
    if (ts.isRegularExpressionLiteral(node) && node.getText().includes('launcher=')) {
      violations.push(`${at()}: the launcher face is gone; ask inShell()`);
    }
    if (file !== appHost && opensWindow(node) && !OTHER_WINDOWS.has(file)) {
      violations.push(`${at()}: an app opens through openApp in ${appHost}`);
    }
    if (file !== appHost && text?.includes('/app/connect/')) {
      violations.push(`${at()}: an app opens through openApp in ${appHost}`);
    }
    if (file !== appTabs && text?.includes('in_launcher')) {
      violations.push(`${at()}: app frames are built in ${appTabs}`);
    }
    if (file !== appTabs && ts.isCallExpression(node) && ts.isIdentifier(node.expression) && node.expression.text === 'setAppHost') {
      violations.push(`${at()}: the tab host is ${appTabs}`);
    }
  });
  return violations;
}

const source = (file: string) => ts.createSourceFile(file, readFileSync(join(root, file), 'utf8'), ts.ScriptTarget.Latest, true);
const fixture = (code: string, file = 'components/Example.tsx') =>
  inspect(ts.createSourceFile(file, code, ts.ScriptTarget.Latest, true), file);

test('the UI asks one question about the desktop shell and opens apps in one place', () => {
  const violations: string[] = [];
  for (const entry of readdirSync(root, { recursive: true, withFileTypes: true })) {
    if (!entry.isFile() || !/\.[cm]?[jt]sx?$/.test(entry.name)) continue;
    const file = relative(root, join(entry.parentPath, entry.name));
    violations.push(...inspect(source(file), file));
  }
  assert.deepEqual(violations, []);
});

test('a second reader of the shell probe fails', () => {
  for (const code of [
    'const ipc = window.__TAURI_INTERNALS__;',
    "const ipc = Reflect.get(window, '__TAURI_INTERNALS__');",
    "if ('__TAURI_INTERNALS__' in window) {}",
  ]) {
    assert.match(fixture(code).join('\n'), /only lib\/appHost.ts asks/);
    assert.deepEqual(fixture(code, appHost), []);
  }
});

test('the launcher query, view and window mark are gone everywhere', () => {
  for (const code of [
    "new URLSearchParams(location.search).get('launcher')",
    "location.search.includes('launcher=1')",
    "if (view === 'launcher') {}",
    "type View = 'main' | 'launcher';",
    "sessionStorage.getItem('linggen:launcher-face')",
    'const face = /[?&]launcher=1/.test(location.search);',
  ]) {
    assert.match(fixture(code).join('\n'), /launcher face is gone/);
    assert.match(fixture(code, appHost).join('\n'), /launcher face is gone/);
  }
  // An app's declared kind (`app.launcher`) and the tab memory key are not the face.
  assert.deepEqual(fixture("apps.filter((a) => a.app.launcher === 'web'); localStorage.getItem('linggen:launcher');"), []);
});

test('a second place deciding where an app opens fails', () => {
  for (const code of [
    "window.open(`/apps/${name}/index.html`, '_blank');",
    "window['open'](url);",
    "globalThis.open(url, '_blank');",
    'const go = `${relay}/app/connect/${id}?app=${encodeURIComponent(url)}`;',
  ]) {
    assert.match(fixture(code).join('\n'), /an app opens through openApp/);
    assert.deepEqual(fixture(code, appHost), []);
  }
  assert.match(fixture('setAppHost(null);').join('\n'), /the tab host is apps\/AppTabs.tsx/);
  assert.deepEqual(fixture('setAppHost(null);', appTabs), []);
  const frame = 'const src = `/apps/${a}/${e}?app_mode=1&in_launcher=1`;';
  assert.match(fixture(frame).join('\n'), /app frames are built in apps\/AppTabs.tsx/);
  assert.deepEqual(fixture(frame, appTabs), []);
  // The named non-app windows stay allowed, and only those files.
  assert.deepEqual(fixture("window.open('/settings?tab=models', '_blank', 'noopener');", 'lib/ownModel.ts'), []);
});
