/**
 * Open a skill app in a new browser tab.
 *
 * Local mode: opens the URL directly.
 * Remote mode: fetches HTML + inlines all resources via WebRTC proxy,
 * recursively resolves ES module imports, opens as blob URL.
 */

const isRemoteMode = typeof document !== 'undefined' && !!document.querySelector('meta[name="linggen-instance"]');

export async function openAppInNewTab(url: string): Promise<void> {
  if (!isRemoteMode || !url.startsWith('/apps/')) {
    window.open(url, '_blank');
    return;
  }

  try {
    const resp = await fetch(url);
    if (!resp.ok) { window.open(url, '_blank'); return; }
    let html = await resp.text();

    const lastSlash = url.lastIndexOf('/');
    const basePath = lastSlash >= 0 ? url.slice(0, lastSlash + 1) : '/';

    // Inline linked stylesheets
    const cssMatches = [...html.matchAll(/<link\s+[^>]*rel=["']stylesheet["'][^>]*href=["']([^"']+)["'][^>]*\/?>/gi)];
    for (const m of cssMatches) {
      if (m[1].startsWith('http')) continue;
      const cssUrl = m[1].startsWith('/') ? m[1] : basePath + m[1];
      try { const r = await fetch(cssUrl); if (r.ok) html = html.replace(m[0], `<style>${await r.text()}</style>`); } catch {}
    }

    // Inline scripts — handle both regular and module scripts
    const jsMatches = [...html.matchAll(/<script\s+([^>]*)src=["']([^"']+)["']([^>]*)><\/script>/gi)];
    for (const m of jsMatches) {
      const src = m[2];
      if (src.startsWith('http')) continue;
      const attrs = m[1] + m[3];
      const isModule = /type=["']module["']/i.test(attrs);
      const jsUrl = src.startsWith('/') ? src : basePath + src;

      try {
        if (isModule) {
          // Recursively resolve ES module imports into a single bundle
          const bundled = await bundleModule(jsUrl, basePath);
          html = html.replace(m[0], `<script type="module">${bundled}<\/script>`);
        } else {
          const r = await fetch(jsUrl);
          if (r.ok) html = html.replace(m[0], `<script>${await r.text()}<\/script>`);
        }
      } catch {}
    }

    // Rewrite remaining relative URLs to absolute
    html = html.replace(/(src|href)=["']((?!http|data:|blob:|#|javascript:)[^"']+)["']/gi,
      (_m, attr, val) => `${attr}="${val.startsWith('/') ? val : basePath + val}"`);

    // Inject a fetch proxy so API calls from the blob page go through WebRTC.
    // The blob page inherits the opener's origin, so window.opener.fetch works.
    const fetchProxyScript = `<script>
if (window.opener && window.opener.fetch) {
  const _openerFetch = window.opener.fetch.bind(window.opener);
  const _origFetch = window.fetch.bind(window);
  window.fetch = function(input, init) {
    const url = typeof input === 'string' ? input : input instanceof URL ? input.toString() : input.url;
    if (typeof url === 'string' && (url.startsWith('/api/') || url.startsWith('/apps/'))) {
      return _openerFetch(input, init);
    }
    return _origFetch(input, init);
  };
}
<\/script>`;
    html = html.replace('<head>', '<head>' + fetchProxyScript);

    const blob = new Blob([html], { type: 'text/html' });
    const blobUrl = URL.createObjectURL(blob);
    window.open(blobUrl, '_blank');
    setTimeout(() => URL.revokeObjectURL(blobUrl), 10000);
  } catch {
    window.open(url, '_blank');
  }
}

/**
 * Recursively fetch and inline all `import ... from './foo.js'` statements
 * in an ES module, producing a single self-contained script.
 */
async function bundleModule(entryUrl: string, basePath: string, visited = new Set<string>()): Promise<string> {
  if (visited.has(entryUrl)) return `/* circular: ${entryUrl} */`;
  visited.add(entryUrl);

  const resp = await fetch(entryUrl);
  if (!resp.ok) return `/* failed to fetch: ${entryUrl} */`;
  let code = await resp.text();

  // Resolve the directory of this module for relative imports
  const lastSlash = entryUrl.lastIndexOf('/');
  const moduleDir = lastSlash >= 0 ? entryUrl.slice(0, lastSlash + 1) : basePath;

  // Find all static imports: import { x } from './api.js'; import './style.js';
  const importPattern = /import\s+(?:(?:\{[^}]*\}|\*\s+as\s+\w+|\w+)(?:\s*,\s*(?:\{[^}]*\}|\*\s+as\s+\w+|\w+))*\s+from\s+)?['"]([^'"]+)['"]\s*;?/g;
  const imports = [...code.matchAll(importPattern)];

  // Process imports bottom-up to preserve string indices
  const inlinedModules: string[] = [];
  for (const imp of imports) {
    const specifier = imp[1];
    // Skip bare specifiers (npm packages) and absolute URLs
    if (!specifier.startsWith('.') && !specifier.startsWith('/')) continue;

    const resolvedUrl = specifier.startsWith('/')
      ? specifier
      : moduleDir + specifier;

    // Recursively bundle the imported module
    const importedCode = await bundleModule(resolvedUrl, basePath, visited);
    inlinedModules.push(importedCode);

    // Replace the import with a comment (exports are now in the same scope)
    code = code.replace(imp[0], `/* inlined: ${specifier} */`);
  }

  // Also strip `export` keywords so everything is in the same scope
  code = code.replace(/export\s+(default\s+)?(?=function|class|const|let|var|async)/g, '');
  code = code.replace(/export\s*\{[^}]*\}\s*;?/g, '');

  // Prepend inlined modules (dependencies first)
  return inlinedModules.join('\n') + '\n' + code;
}
