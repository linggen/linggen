import React, { useEffect, useState, useRef, useCallback } from 'react';
import { X } from 'lucide-react';

export interface AppPanelState {
  skill: string;
  launcher: string;
  url: string;
  title: string;
  width?: number;
  height?: number;
}

/** Detect remote mode (linggen.dev relay page). */
const isRemoteMode = typeof document !== 'undefined' && !!document.querySelector('meta[name="linggen-instance"]');

/**
 * Fetch an app page via the WebRTC fetch proxy, inline all CSS/JS,
 * and inject a navigation interceptor for multi-page apps.
 */
async function fetchAndInlineAppHtml(url: string): Promise<string | null> {
  try {
    const resp = await fetch(url);
    if (!resp.ok) return null;
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

    // Inline scripts (both regular and module)
    const jsMatches = [...html.matchAll(/<script\s+([^>]*)src=["']([^"']+)["']([^>]*)><\/script>/gi)];
    for (const m of jsMatches) {
      const src = m[2];
      if (src.startsWith('http')) continue;
      const attrs = m[1] + m[3];
      const isModule = /type=["']module["']/i.test(attrs);
      const jsUrl = src.startsWith('/') ? src : basePath + src;
      try {
        if (isModule) {
          const bundled = await bundleModule(jsUrl, basePath);
          html = html.replace(m[0], `<script type="module">${bundled}<\/script>`);
        } else {
          const r = await fetch(jsUrl);
          if (r.ok) html = html.replace(m[0], `<script>${await r.text()}<\/script>`);
        }
      } catch {}
    }

    // Rewrite remaining relative URLs to absolute
    html = html.replace(/(src|href)=["']((?!http|data:|blob:|#|javascript:|mailto:)[^"']+)["']/gi,
      (_m, attr, val) => `${attr}="${val.startsWith('/') ? val : basePath + val}"`);

    // Inject navigation interceptor + fetch proxy at the top of <head>.
    // This intercepts window.location.href assignments and sends a postMessage
    // to the parent so it can re-fetch and re-inline the target page.
    const injectedScript = `<script>
(function() {
  // Proxy fetch calls through opener/parent which has WebRTC fetch proxy
  var _parentFetch = (window.parent !== window && window.parent.fetch)
    ? window.parent.fetch.bind(window.parent) : null;
  if (_parentFetch) {
    var _origFetch = window.fetch.bind(window);
    window.fetch = function(input, init) {
      var url = typeof input === 'string' ? input : input instanceof URL ? input.toString() : input.url;
      if (typeof url === 'string' && (url.startsWith('/api/') || url.startsWith('/apps/'))) {
        return _parentFetch(input, init);
      }
      return _origFetch(input, init);
    };
  }
  // Intercept navigation for multi-page apps
  var _basePath = ${JSON.stringify(basePath)};
  var origAssign = Object.getOwnPropertyDescriptor(Location.prototype, 'href');
  if (origAssign && origAssign.set) {
    Object.defineProperty(window.location, 'href', {
      set: function(val) {
        if (val && !val.startsWith('http') && !val.startsWith('blob:') && !val.startsWith('javascript:')) {
          var absUrl = val.startsWith('/') ? val : _basePath + val;
          window.parent.postMessage({ type: 'linggen-app-navigate', url: absUrl }, '*');
          return;
        }
        origAssign.set.call(this, val);
      },
      get: origAssign.get ? function() { return origAssign.get.call(this); } : undefined,
      configurable: true,
    });
  }
})();
<\/script>`;
    html = html.replace('<head>', '<head>' + injectedScript);

    return html;
  } catch {
    return null;
  }
}

/** Recursively resolve ES module imports into a single inline bundle. */
async function bundleModule(entryUrl: string, basePath: string, visited = new Set<string>()): Promise<string> {
  if (visited.has(entryUrl)) return '';
  visited.add(entryUrl);
  const resp = await fetch(entryUrl);
  if (!resp.ok) return '';
  let code = await resp.text();
  const lastSlash = entryUrl.lastIndexOf('/');
  const moduleDir = lastSlash >= 0 ? entryUrl.slice(0, lastSlash + 1) : basePath;

  const importPattern = /import\s+(?:(?:\{[^}]*\}|\*\s+as\s+\w+|\w+)(?:\s*,\s*(?:\{[^}]*\}|\*\s+as\s+\w+|\w+))*\s+from\s+)?['"]([^'"]+)['"]\s*;?/g;
  const imports = [...code.matchAll(importPattern)];
  const inlined: string[] = [];
  for (const imp of imports) {
    const spec = imp[1];
    if (!spec.startsWith('.') && !spec.startsWith('/')) continue;
    const resolved = spec.startsWith('/') ? spec : moduleDir + spec;
    inlined.push(await bundleModule(resolved, basePath, visited));
    code = code.replace(imp[0], `/* inlined: ${spec} */`);
  }
  code = code.replace(/export\s+(default\s+)?(?=function|class|const|let|var|async)/g, '');
  code = code.replace(/export\s*\{[^}]*\}\s*;?/g, '');
  return inlined.join('\n') + '\n' + code;
}

export const AppPanel: React.FC<{
  app: AppPanelState;
  onClose: () => void;
}> = ({ app, onClose }) => {
  const [srcdoc, setSrcdoc] = useState<string | null>(null);
  const iframeRef = useRef<HTMLIFrameElement>(null);

  // Load the app page (remote: fetch+inline, local: not used — direct src)
  const loadPage = useCallback(async (url: string) => {
    if (!isRemoteMode) return;
    setSrcdoc(null); // show loading
    const html = await fetchAndInlineAppHtml(url);
    if (html) setSrcdoc(html);
  }, []);

  useEffect(() => { loadPage(app.url); }, [app.url, loadPage]);

  // Listen for navigation messages from the iframe (multi-page app navigation)
  useEffect(() => {
    if (!isRemoteMode) return;
    const handler = (e: MessageEvent) => {
      if (e.data?.type === 'linggen-app-navigate' && e.data.url) {
        loadPage(e.data.url);
      }
    };
    window.addEventListener('message', handler);
    return () => window.removeEventListener('message', handler);
  }, [loadPage]);

  return (
    <div className="fixed inset-0 z-50 flex flex-col bg-white dark:bg-zinc-900">
      <div className="flex items-center justify-between px-4 py-2 border-b border-slate-200 dark:border-white/10 bg-slate-50 dark:bg-zinc-800/50 shrink-0">
        <div className="flex items-center gap-2">
          <span className="text-sm font-semibold text-slate-700 dark:text-slate-200">{app.title}</span>
          <span className="text-[10px] font-mono text-slate-400 dark:text-slate-500">{app.skill}</span>
        </div>
        <button
          onClick={onClose}
          className="p-1.5 hover:bg-slate-200 dark:hover:bg-white/10 rounded-lg transition-colors text-slate-400 hover:text-slate-600 dark:hover:text-slate-300"
        >
          <X size={16} />
        </button>
      </div>
      <div className="flex-1 min-h-0">
        {isRemoteMode ? (
          srcdoc ? (
            <iframe
              ref={iframeRef}
              srcDoc={srcdoc}
              title={app.title}
              style={{ width: '100%', height: '100%', border: 'none' }}
              sandbox="allow-scripts allow-same-origin allow-popups allow-forms"
            />
          ) : (
            <div className="flex items-center justify-center h-full text-sm text-slate-400">
              Loading app...
            </div>
          )
        ) : (
          <iframe
            src={app.url}
            title={app.title}
            style={{ width: '100%', height: '100%', border: 'none' }}
            sandbox="allow-scripts allow-same-origin allow-popups allow-forms"
          />
        )}
      </div>
    </div>
  );
};
