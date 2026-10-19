/**
 * The engine's pages in the one language the person set (Settings → General →
 * Language, `/api/user/language`).
 *
 * Components keep writing English. The page's English is the key: every text
 * node, title, placeholder and aria-label that matches an entry in
 * strings-zh.ts is put into the person's language as it lands, and a `{slot}`
 * in an entry matches any run of text ("{n} sessions"). Model and user text in
 * the chat never matches by accident — the transcript, editors and inputs are
 * skipped outright, and so is anything marked `data-no-i18n`.
 *
 * Read once at start, before the first render; a change in Settings reloads
 * the page into the new language.
 */
import { ZH } from './strings-zh';

export type Language = 'en' | 'zh-Hans';
let lang: Language = 'en';

export const language = (): Language => lang;

/** "zh", "zh-CN", "zh-Hans-CN", "zh-TW"… → zh-Hans; anything else → en. */
export function normalizeLanguage(tag: string | null | undefined): Language {
  const s = String(tag || '').trim().toLowerCase().replace(/_/g, '-');
  return s === 'zh' || s.startsWith('zh-') ? 'zh-Hans' : 'en';
}

/** The engine's effective value, else the browser's; `?lang=` wins for a check. */
export async function loadLanguage(): Promise<Language> {
  let tag = new URLSearchParams(location.search).get('lang') || '';
  if (!tag) {
    try {
      const res = await fetch('/api/user/language', { cache: 'no-store' });
      if (res.ok) tag = (await res.json())?.language || '';
    } catch { /* an engine without the endpoint — fall back */ }
  }
  if (!tag) tag = navigator.language || '';
  lang = normalizeLanguage(tag);
  document.documentElement.lang = lang;
  return lang;
}

// ── The page's English, put into the person's language in place ───────────

const SKIP =
  'script, style, textarea, input, pre, code, [contenteditable], .cm-editor, ' +
  '[data-no-i18n], .markdown-body, .chat-message-content';
const ATTRS = ['title', 'placeholder', 'aria-label'];

let patterns: { re: RegExp; names: string[]; zh: string }[] | null = null;

function index() {
  patterns = [];
  for (const [en, zh] of Object.entries(ZH)) {
    const names = [...en.matchAll(/\{(\w+)\}/g)].map((m) => m[1]);
    if (!names.length) continue;
    const body = en
      .split(/\{\w+\}/)
      .map((s) => s.replace(/[.*+?^${}()|[\]\\]/g, '\\$&'))
      .join('(.+?)');
    patterns.push({ re: new RegExp(`^${body}$`), names, zh });
  }
}

/** The person's words for one piece of the page's English, or null. */
export function translate(text: string): string | null {
  if (lang === 'en') return null;
  const s = text.trim();
  if (!s) return null;
  const hit = ZH[s];
  if (hit !== undefined) return hit;
  if (!patterns) index();
  for (const p of patterns!) {
    const m = p.re.exec(s);
    if (m) return p.zh.replace(/\{(\w+)\}/g, (_, k: string) => m[p.names.indexOf(k) + 1] ?? `{${k}}`);
  }
  return null;
}

function swapText(node: Text) {
  const v = node.nodeValue || '';
  const said = translate(v);
  if (said == null) return;
  node.nodeValue = (v.match(/^\s*/)?.[0] ?? '') + said + (v.match(/\s*$/)?.[0] ?? '');
}

function swapAttrs(el: Element) {
  for (const a of ATTRS) {
    const v = el.getAttribute(a);
    if (!v) continue;
    const said = translate(v);
    if (said != null) el.setAttribute(a, said);
  }
}

export function translateDom(root: Node = document.body) {
  if (lang === 'en' || !root) return;
  if (root.nodeType === Node.TEXT_NODE) {
    if (!root.parentElement?.closest(SKIP)) swapText(root as Text);
    return;
  }
  if (root.nodeType !== Node.ELEMENT_NODE || (root as Element).closest(SKIP)) return;
  swapAttrs(root as Element);
  const walk = document.createTreeWalker(root, NodeFilter.SHOW_ELEMENT | NodeFilter.SHOW_TEXT, {
    acceptNode: (n) =>
      n.nodeType === Node.ELEMENT_NODE && (n as Element).matches(SKIP)
        ? NodeFilter.FILTER_REJECT
        : NodeFilter.FILTER_ACCEPT,
  });
  for (let n = walk.nextNode(); n; n = walk.nextNode()) {
    if (n.nodeType === Node.TEXT_NODE) swapText(n as Text);
    else swapAttrs(n as Element);
  }
}

/** Translate now, and everything React renders from here on. */
export function watchDom(root: Node = document.body) {
  if (lang === 'en') return;
  translateDom(root);
  new MutationObserver((muts) => {
    for (const m of muts) {
      if (m.type === 'characterData') translateDom(m.target);
      else if (m.type === 'attributes') swapAttrs(m.target as Element);
      else m.addedNodes.forEach((n) => translateDom(n));
    }
  }).observe(root, { childList: true, subtree: true, characterData: true, attributes: true, attributeFilter: ATTRS });
}
