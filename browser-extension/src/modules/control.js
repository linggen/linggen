// Control module — browser control per linggen/doc/browser-control-spec.md.
//
// Drives ONE visible controlled tab through the Chrome DevTools Protocol
// (chrome.debugger): input via `Input`, capture via `Page`, the node tree via
// `Accessibility`/`DOM`. Real events and true screenshots — the same path
// Claude in Chrome and Playwright use. Attaching surfaces Chrome's "started
// debugging this browser" banner; that is expected and disclosed.
//
// Targeting is reference-first: `read_page` returns a tree where each
// actionable node carries a stable ref ("n42") mapped here to its CDP
// backendDOMNodeId. `click`/`type` resolve refs back to real coordinates.
// Screenshot + pixel coordinates is the fallback for what the tree can't
// express. The read modules' hidden throwaway tabs are untouched — control
// is a separate, visible surface.

import { gateAction } from "../gate.js";

const CDP_VERSION = "1.3";
const NAVIGATE_TIMEOUT_MS = 30000;
const WAIT_SELECTOR_TIMEOUT_MS = 10000;
const WAIT_MS_CAP = 10000;
const CONSOLE_BUFFER_CAP = 300;
const MAX_TREE_NODES = 400;

function err(code, message) {
  const e = new Error(message);
  e.code = code;
  return e;
}

const sleep = (ms) => new Promise((r) => setTimeout(r, ms));

// ── controlled tab + debugger session ───────────────────────────────────────

let controlledTabId = null;
let attached = false;
// ref ("n42") -> { backendDOMNodeId, role, name }
let refMap = new Map();
let refSeq = 0;
// Ring buffer of console entries from the controlled tab.
let consoleBuf = [];

chrome.debugger.onDetach.addListener((source) => {
  if (source.tabId === controlledTabId) attached = false;
});

chrome.tabs.onRemoved.addListener((tabId) => {
  if (tabId === controlledTabId) {
    controlledTabId = null;
    attached = false;
    refMap.clear();
    consoleBuf = [];
  }
});

chrome.debugger.onEvent.addListener((source, method, params) => {
  if (source.tabId !== controlledTabId) return;
  if (method === "Runtime.consoleAPICalled") {
    pushConsole({
      level: params.type || "log",
      text: (params.args || [])
        .map((a) => (a.value !== undefined ? String(a.value) : a.description || a.type))
        .join(" "),
      ts: params.timestamp,
    });
  } else if (method === "Log.entryAdded" && params.entry) {
    pushConsole({
      level: params.entry.level || "log",
      text: params.entry.text || "",
      source: params.entry.source,
      ts: params.entry.timestamp,
    });
  }
});

function pushConsole(entry) {
  consoleBuf.push(entry);
  if (consoleBuf.length > CONSOLE_BUFFER_CAP) consoleBuf.shift();
}

// chrome.debugger.sendCommand does NOT always call back — a target that is
// paused, navigating across a process boundary, or blocked behind a page
// dialog never answers. Unbounded, one such command hangs its op, and the op
// hangs the whole serialized control queue behind it: every later call then
// dies at its caller's timeout and the only way back is reloading the
// extension. Observed 2026-08-20 driving x.com. Bound every command.
const COMMAND_TIMEOUT_MS = 10000;

function cdp(method, params = {}) {
  return new Promise((resolve, reject) => {
    let settled = false;
    const timer = setTimeout(() => {
      if (settled) return;
      settled = true;
      reject(err("upstream_error", `${method}: the page did not answer in ${COMMAND_TIMEOUT_MS / 1000}s`));
    }, COMMAND_TIMEOUT_MS);
    chrome.debugger.sendCommand({ tabId: controlledTabId }, method, params, (result) => {
      if (settled) return;
      settled = true;
      clearTimeout(timer);
      const e = chrome.runtime.lastError;
      if (e) reject(err("upstream_error", `${method}: ${e.message}`));
      else resolve(result || {});
    });
  });
}

async function tabInfo() {
  if (controlledTabId == null) return null;
  try {
    return await chrome.tabs.get(controlledTabId);
  } catch {
    controlledTabId = null;
    attached = false;
    return null;
  }
}

/// Put the controlled tab in its own named tab group, so the one tab an agent
/// drives is obvious in the strip and can't be mistaken for the user's own.
///
/// Cosmetic by design: a failure here must never take browser control down
/// with it. Nothing to tear down either — Chrome dissolves a group when its
/// last tab closes, and we always create our own tab.
const MARK_TAB_TIMEOUT_MS = 1500;

async function markTab(tabId) {
  // Bounded — see x.js. Not every Chromium browser has tab groups (Arc has
  // Spaces), and there `chrome.tabs.group` can never call back. A try/catch
  // cannot rescue a call that never returns, and this runs before the debugger
  // attaches, so a hang here stalls every control op behind it.
  const marked = (async () => {
    const groupId = await chrome.tabs.group({ tabIds: tabId });
    await chrome.tabGroups.update(groupId, { title: "Linggen", color: "green" });
  })();
  try {
    await Promise.race([marked, sleep(MARK_TAB_TIMEOUT_MS)]);
  } catch (e) {
    console.warn("[control] could not group the controlled tab:", e);
  }
}

/// Create (or reuse) the visible controlled tab and attach the debugger.
async function ensureControlledTab(url) {
  let tab = await tabInfo();
  if (!tab) {
    tab = await chrome.tabs.create({ url: url || "about:blank", active: true });
    controlledTabId = tab.id;
    attached = false;
    refMap.clear();
    consoleBuf = [];
    await markTab(tab.id);
  }
  if (!attached) {
    await new Promise((resolve, reject) => {
      const timer = setTimeout(
        () => reject(err("upstream_error", "debugger attach did not answer")),
        COMMAND_TIMEOUT_MS,
      );
      chrome.debugger.attach({ tabId: controlledTabId }, CDP_VERSION, () => {
        clearTimeout(timer);
        const e = chrome.runtime.lastError;
        // "Already attached" from a previous worker life is fine.
        if (e && !/already attached/i.test(e.message)) {
          reject(err("upstream_error", `debugger attach: ${e.message}`));
        } else {
          resolve();
        }
      });
    });
    attached = true;
    await verifyAttachedTo(controlledTabId);
    await cdp("Page.enable");
    await cdp("Runtime.enable");
    await cdp("Log.enable").catch(() => {});
  }
  return controlledTabId;
}

/// Confirm the debugger session answers FOR THE TAB WE ASKED FOR.
///
/// Liveness is not identity. A session on the wrong tab answers everything and
/// reports that nothing we look for exists, so every deadline burns hunting
/// elements on a page it was never pointed at. Arc resolves extension tab ids
/// to a different tab than the one opened, which parked sessions on Linggen's
/// own UI — a page an agent must never be driving.
async function verifyAttachedTo(tabId) {
  let href = "";
  try {
    const res = await cdp("Runtime.evaluate", {
      expression: "location.href",
      returnByValue: true,
    });
    href = (res && res.result && res.result.value) || "";
  } catch (e) {
    attached = false;
    await new Promise((r) => chrome.debugger.detach({ tabId }, () => { void chrome.runtime.lastError; r(); }));
    throw err("upstream_error", `the debugger attached but the tab does not answer it (${e.message})`);
  }
  let expected = "";
  try {
    const tab = await chrome.tabs.get(tabId);
    expected = tab.url || "";
  } catch (e) { /* gone; the href alone has to do */ }
  if (href && expected && originOf(href) !== originOf(expected)) {
    attached = false;
    await new Promise((r) => chrome.debugger.detach({ tabId }, () => { void chrome.runtime.lastError; r(); }));
    throw err(
      "upstream_error",
      `the debugger attached to the wrong page — asked for ${expected}, but the ` +
      `session answers for ${href}. This browser maps extension tab ids to a ` +
      "different tab than the one that was opened.",
    );
  }
}

function requireTab() {
  if (controlledTabId == null) {
    throw err("bad_request", "no controlled tab — call navigate (or tabs open) first");
  }
}

function originOf(url) {
  try {
    return new URL(url).origin;
  } catch {
    return null;
  }
}

/// Origin of the controlled tab right now — what mutating actions gate on.
async function currentOrigin() {
  const tab = await tabInfo();
  return tab && tab.url ? originOf(tab.url) : null;
}

// ── navigation ──────────────────────────────────────────────────────────────

/// Resolve once the tab reports `complete`, plus a short settle for SPA paint.
function waitForLoad(timeoutMs = NAVIGATE_TIMEOUT_MS) {
  return new Promise((resolve) => {
    let done = false;
    const finish = () => {
      if (done) return;
      done = true;
      chrome.tabs.onUpdated.removeListener(listener);
      setTimeout(resolve, 500); // settle: let the first paint/JS tick land
    };
    const listener = (tabId, info) => {
      if (tabId === controlledTabId && info.status === "complete") finish();
    };
    chrome.tabs.onUpdated.addListener(listener);
    // Already complete (SPA route change, cached page) or hard timeout.
    tabInfo().then((t) => { if (t && t.status === "complete") finish(); });
    setTimeout(finish, timeoutMs);
  });
}

async function opNavigate(params) {
  const target = params.url;
  if (!target) throw err("bad_request", "navigate requires url (or \"back\"/\"forward\")");

  if (target === "back" || target === "forward") {
    // Moving within history the user already watched — no gate.
    requireTab();
    await ensureControlledTab();
    const hist = await cdp("Page.getNavigationHistory");
    const idx = hist.currentIndex + (target === "back" ? -1 : 1);
    const entry = hist.entries[idx];
    if (!entry) throw err("bad_request", `no history entry to go ${target} to`);
    await cdp("Page.navigateToHistoryEntry", { entryId: entry.id });
  } else {
    await gateAction({ origin: originOf(target), summary: `Navigate to ${target}` });
    const first = controlledTabId == null;
    await ensureControlledTab(target);
    if (!first) await chrome.tabs.update(controlledTabId, { url: target, active: true });
  }
  refMap.clear();
  await waitForLoad();
  const tab = await tabInfo();
  return { url: tab?.url || "", title: tab?.title || "" };
}

// ── read_page: accessibility tree with refs ────────────────────────────────

const INTERACTIVE_ROLES = new Set([
  "button", "link", "textbox", "searchbox", "checkbox", "radio", "combobox",
  "listbox", "option", "menuitem", "menuitemcheckbox", "menuitemradio", "tab",
  "switch", "slider", "spinbutton",
]);

const STRUCTURAL_ROLES = new Set([
  "heading", "img", "image", "list", "listitem", "table", "row", "cell",
  "form", "dialog", "alert", "navigation", "main", "article", "banner",
  "region", "tabpanel", "menu", "toolbar", "paragraph",
]);

function axValue(v) {
  return v && v.value != null ? String(v.value) : "";
}

async function opReadPage(params) {
  requireTab();
  await ensureControlledTab();
  await cdp("Accessibility.enable").catch(() => {});
  const { nodes } = await cdp("Accessibility.getFullAXTree");
  await cdp("Accessibility.disable").catch(() => {});

  const byId = new Map(nodes.map((n) => [n.nodeId, n]));
  const root = nodes.find((n) => !n.parentId) || nodes[0];
  if (!root) throw err("upstream_error", "empty accessibility tree");

  refMap.clear();
  refSeq = 0;
  const lines = [];
  const outNodes = [];
  const interactiveOnly = params.filter === "interactive";
  let emitted = 0;

  const walk = (node, depth) => {
    if (!node || emitted >= MAX_TREE_NODES) return;
    const role = axValue(node.role);
    const name = axValue(node.name).trim().replace(/\s+/g, " ").slice(0, 120);
    const ignored = node.ignored === true;
    const interactive = INTERACTIVE_ROLES.has(role);
    const structural = STRUCTURAL_ROLES.has(role);
    const isText = role === "StaticText" || role === "text";

    let show = false;
    if (!ignored) {
      if (interactive) show = true;
      else if (!interactiveOnly && (structural || (isText && name))) show = true;
    }

    if (show) {
      emitted += 1;
      let line = "  ".repeat(Math.min(depth, 12));
      if (interactive && node.backendDOMNodeId != null) {
        refSeq += 1;
        const ref = `n${refSeq}`;
        refMap.set(ref, { backendDOMNodeId: node.backendDOMNodeId, role, name });
        line += `[${ref}] `;
        outNodes.push({ ref, role, name });
      }
      line += role;
      if (name) line += ` "${name}"`;
      const value = axValue(node.value);
      if (value && value !== name) line += ` value="${value.slice(0, 80)}"`;
      lines.push(line);
    }

    const nextDepth = show ? depth + 1 : depth;
    for (const childId of node.childIds || []) {
      walk(byId.get(childId), nextDepth);
    }
  };
  walk(root, 0);

  const tab = await tabInfo();
  return {
    url: tab?.url || "",
    title: tab?.title || "",
    truncated: emitted >= MAX_TREE_NODES,
    nodes: outNodes,
    tree: lines.join("\n"),
  };
}

// ── screenshot ──────────────────────────────────────────────────────────────

async function opScreenshot(params) {
  requireTab();
  await ensureControlledTab();
  const args = { format: "png" };
  const r = params.region;
  if (r && [r.x, r.y, r.width, r.height].every((v) => typeof v === "number")) {
    args.clip = { x: r.x, y: r.y, width: r.width, height: r.height, scale: 1 };
  }
  const { data } = await cdp("Page.captureScreenshot", args);
  const tab = await tabInfo();
  return { url: tab?.url || "", base64: data };
}

// ── ref resolution + input ──────────────────────────────────────────────────

/// ref → viewport center point. Scrolls the node into view first.
async function refToPoint(ref) {
  const entry = refMap.get(ref);
  if (!entry) throw err("element_gone", `unknown ref '${ref}' — re-run read_page`);
  const node = { backendNodeId: entry.backendDOMNodeId };
  try {
    await cdp("DOM.enable");
    await cdp("DOM.getDocument", { depth: 0 });
    await cdp("DOM.scrollIntoViewIfNeeded", node).catch(() => {});
    const { model } = await cdp("DOM.getBoxModel", node);
    const q = model.content; // [x1,y1, x2,y2, x3,y3, x4,y4]
    return {
      x: (q[0] + q[2] + q[4] + q[6]) / 4,
      y: (q[1] + q[3] + q[5] + q[7]) / 4,
      entry,
    };
  } catch (e) {
    if (e.code === "element_gone") throw e;
    throw err("element_gone", `ref '${ref}' no longer resolves (${e.message}) — re-run read_page`);
  }
}

const MOUSE_BUTTONS = { left: 1, middle: 4, right: 2 };

async function mouseClick(x, y, button = "left", clickCount = 1) {
  const base = { x, y, button, buttons: MOUSE_BUTTONS[button] || 1, clickCount };
  await cdp("Input.dispatchMouseEvent", { type: "mouseMoved", x, y });
  await cdp("Input.dispatchMouseEvent", { type: "mousePressed", ...base });
  await cdp("Input.dispatchMouseEvent", { type: "mouseReleased", ...base });
}

async function opClick(params) {
  requireTab();
  await ensureControlledTab();
  let x, y, target = null;
  if (params.ref) {
    const p = await refToPoint(params.ref);
    x = p.x; y = p.y; target = p.entry;
  } else if (params.coordinate && params.coordinate.length === 2) {
    [x, y] = params.coordinate;
  } else {
    throw err("bad_request", "click requires ref or coordinate [x,y]");
  }
  await gateAction({
    origin: await currentOrigin(),
    targetName: target ? target.name : "",
    summary: `Click ${target ? `${target.role} "${target.name}"` : `coordinate (${Math.round(x)},${Math.round(y)})`}`,
  });
  await mouseClick(x, y, params.button || "left", params.double ? 2 : 1);
  await sleep(300); // let click handlers run before the next read
  const tab = await tabInfo();
  return {
    clicked: true,
    target: target ? `${target.role} "${target.name}"` : `(${Math.round(x)},${Math.round(y)})`,
    url: tab?.url || "",
  };
}

async function opType(params) {
  requireTab();
  await ensureControlledTab();
  if (typeof params.text !== "string") throw err("bad_request", "type requires text");
  let point = null;
  if (params.ref) point = await refToPoint(params.ref);
  const preview = params.text.slice(0, 60);
  await gateAction({
    origin: await currentOrigin(),
    targetName: point ? point.entry.name : "",
    summary: `Type "${preview}" into ${point ? `${point.entry.role} "${point.entry.name}"` : "the focused field"}`,
  });
  if (point) {
    await mouseClick(point.x, point.y); // focus the field the human way
    await sleep(150);
    if (params.clear) {
      await keyChord("Meta+a").catch(() => {});
      await keyChord("Control+a").catch(() => {});
      await keyChord("Backspace");
    }
  }
  await cdp("Input.insertText", { text: params.text });
  return { typed: params.text.length };
}

// Common non-printable keys: DOM key → { code, keyCode }.
const KEY_DEFS = {
  enter: { key: "Enter", code: "Enter", keyCode: 13, text: "\r" },
  tab: { key: "Tab", code: "Tab", keyCode: 9 },
  escape: { key: "Escape", code: "Escape", keyCode: 27 },
  backspace: { key: "Backspace", code: "Backspace", keyCode: 8 },
  delete: { key: "Delete", code: "Delete", keyCode: 46 },
  arrowup: { key: "ArrowUp", code: "ArrowUp", keyCode: 38 },
  arrowdown: { key: "ArrowDown", code: "ArrowDown", keyCode: 40 },
  arrowleft: { key: "ArrowLeft", code: "ArrowLeft", keyCode: 37 },
  arrowright: { key: "ArrowRight", code: "ArrowRight", keyCode: 39 },
  home: { key: "Home", code: "Home", keyCode: 36 },
  end: { key: "End", code: "End", keyCode: 35 },
  pageup: { key: "PageUp", code: "PageUp", keyCode: 33 },
  pagedown: { key: "PageDown", code: "PageDown", keyCode: 34 },
  space: { key: " ", code: "Space", keyCode: 32, text: " " },
};

const MODIFIER_BITS = { alt: 1, control: 2, ctrl: 2, meta: 4, cmd: 4, command: 4, shift: 8 };

/// "Ctrl+Shift+Enter" / "a" → CDP key events.
async function keyChord(chord) {
  const parts = String(chord).split("+").map((s) => s.trim()).filter(Boolean);
  const keyPart = parts[parts.length - 1];
  let modifiers = 0;
  for (const p of parts.slice(0, -1)) {
    const bit = MODIFIER_BITS[p.toLowerCase()];
    if (bit == null) throw err("bad_request", `unknown modifier '${p}'`);
    modifiers |= bit;
  }
  const def = KEY_DEFS[keyPart.toLowerCase()] ||
    (keyPart.length === 1
      ? { key: keyPart, code: `Key${keyPart.toUpperCase()}`, keyCode: keyPart.toUpperCase().charCodeAt(0), text: keyPart }
      : null);
  if (!def) throw err("bad_request", `unknown key '${keyPart}'`);

  const down = {
    type: def.text && !modifiers ? "keyDown" : "rawKeyDown",
    key: def.key, code: def.code, modifiers,
    windowsVirtualKeyCode: def.keyCode, nativeVirtualKeyCode: def.keyCode,
  };
  if (def.text && !modifiers) down.text = def.text;
  await cdp("Input.dispatchKeyEvent", down);
  await cdp("Input.dispatchKeyEvent", { type: "keyUp", key: def.key, code: def.code, modifiers, windowsVirtualKeyCode: def.keyCode, nativeVirtualKeyCode: def.keyCode });
}

async function opKey(params) {
  requireTab();
  await ensureControlledTab();
  if (!params.keys) throw err("bad_request", "key requires keys (e.g. \"Enter\", \"Ctrl+a\")");
  await gateAction({ origin: await currentOrigin(), summary: `Press ${params.keys}` });
  const repeat = Math.min(params.repeat || 1, 20);
  for (let i = 0; i < repeat; i++) await keyChord(params.keys);
  return { pressed: params.keys, repeat };
}

async function opScroll(params) {
  requireTab();
  await ensureControlledTab();
  const amount = Math.min(params.amount || 600, 5000);
  const deltas = {
    down: [0, amount], up: [0, -amount], right: [amount, 0], left: [-amount, 0],
  }[params.direction || "down"];
  if (!deltas) throw err("bad_request", `unknown direction '${params.direction}'`);

  let x = 400, y = 300;
  if (params.ref) {
    const p = await refToPoint(params.ref);
    x = p.x; y = p.y;
  } else {
    const tab = await tabInfo();
    if (tab && tab.width) { x = tab.width / 2; y = (tab.height || 600) / 2; }
  }
  await cdp("Input.dispatchMouseEvent", {
    type: "mouseWheel", x, y, deltaX: deltas[0], deltaY: deltas[1],
  });
  await sleep(200);
  return { scrolled: params.direction || "down", amount };
}

async function opWait(params) {
  requireTab();
  await ensureControlledTab();
  switch (params.for) {
    case "load":
      await waitForLoad();
      return { waited: "load" };
    case "ms": {
      const ms = Math.min(Number(params.value) || 0, WAIT_MS_CAP);
      await sleep(ms);
      return { waited: `${ms}ms` };
    }
    case "selector": {
      const sel = String(params.value || "");
      if (!sel) throw err("bad_request", "wait selector requires value");
      const deadline = Date.now() + WAIT_SELECTOR_TIMEOUT_MS;
      while (Date.now() < deadline) {
        const { result } = await cdp("Runtime.evaluate", {
          expression: `!!document.querySelector(${JSON.stringify(sel)})`,
          returnByValue: true,
        });
        if (result && result.value === true) return { waited: "selector", found: true };
        await sleep(250);
      }
      return { waited: "selector", found: false };
    }
    default:
      throw err("bad_request", "wait requires for: load | selector | ms");
  }
}

async function opReadConsole(params) {
  requireTab();
  const limit = Math.min(params.limit || 50, CONSOLE_BUFFER_CAP);
  return { messages: consoleBuf.slice(-limit) };
}

// v1 manages a single controlled tab (multi-tab orchestration is deferred).
async function opTabs(params) {
  switch (params.action || "list") {
    case "list": {
      const tab = await tabInfo();
      return { tabs: tab ? [{ url: tab.url || "", title: tab.title || "", controlled: true, active: !!tab.active }] : [] };
    }
    case "open":
      return opNavigate({ url: params.url || "about:blank" });
    case "switch": {
      requireTab();
      await chrome.tabs.update(controlledTabId, { active: true });
      const tab = await tabInfo();
      return { url: tab?.url || "", title: tab?.title || "" };
    }
    case "close": {
      const id = controlledTabId;
      if (id != null) await chrome.tabs.remove(id).catch(() => {});
      controlledTabId = null;
      attached = false;
      refMap.clear();
      return { closed: id != null };
    }
    default:
      throw err("bad_request", `unknown tabs action '${params.action}'`);
  }
}

export const controlModule = {
  version: "5",
  // Control ops are user-visible interactions in a visible tab — they must
  // stay responsive, so the bridge core skips the anti-bot pacing queue.
  unpaced: true,
  ops: {
    navigate: opNavigate,
    read_page: opReadPage,
    screenshot: opScreenshot,
    click: opClick,
    type: opType,
    key: opKey,
    scroll: opScroll,
    wait: opWait,
    read_console: opReadConsole,
    tabs: opTabs,
  },
};
