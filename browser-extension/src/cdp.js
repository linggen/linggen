// Tab-scoped CDP glue — attach the debugger to ONE tab and drive it.
//
// control.js carries its own copy of this, shaped around the single controlled
// tab it owns (module-level tab id, a11y refs). This one takes a tabId per
// call so a module that opens its own tab — x.js's `post` — can drive it
// without borrowing control's, which the two bridge queues would otherwise
// race over: control ops are serialized on one chain, x ops paced on another,
// and nothing coordinates the two.
//
// Input goes through Input.dispatchMouseEvent / Input.dispatchKeyEvent, which
// enter Chrome's pipeline in the browser process, so the page receives events
// with isTrusted:true — indistinguishable from a hand at the event level.
// Page-injected `new MouseEvent(...)` is isTrusted:false and detectable in one
// line, which is why nothing here synthesizes events from page script.

const CDP_VERSION = "1.3";

// Movement and typing are dispatched as many small events rather than one jump,
// because a pointer that teleports to a button and text that appears whole are
// the parts of automation a page CAN see. This is not a disguise — the events
// are real — it just declines to hand over a free tell.
const MOVE_STEPS_MIN = 18;
const MOVE_STEPS_MAX = 34;
const MOVE_STEP_MS_MIN = 6;
const MOVE_STEP_MS_MAX = 18;
const TYPE_MS_MIN = 40;
const TYPE_MS_MAX = 130;

export function sleep(ms) {
  return new Promise((r) => setTimeout(r, ms));
}

function jitter(min, max) {
  return min + Math.random() * (max - min);
}

function originOf(url) {
  try {
    return new URL(url).origin;
  } catch {
    return null;
  }
}

function err(code, message) {
  const e = new Error(message);
  e.code = code;
  return e;
}

// chrome.debugger.sendCommand does NOT always call back. A target that is
// paused, navigating across a process boundary, or blocked behind a page
// dialog simply never answers — and an unbounded await here is worse than an
// error, because every deadline above it is a `while (Date.now() < deadline)`
// that never gets to re-check its clock. One hung command then hangs the op,
// and the op hangs the serialized queue behind it. Every CDP call is bounded.
const COMMAND_TIMEOUT_MS = 10000;
// The attach probe is deliberately short: it is one trivial round-trip, and
// its whole job is to fail FAST when the session is dead.
const ATTACH_PROBE_MS = 3000;

export function send(tabId, method, params = {}, timeoutMs = COMMAND_TIMEOUT_MS) {
  return new Promise((resolve, reject) => {
    let settled = false;
    const timer = setTimeout(() => {
      if (settled) return;
      settled = true;
      reject(err("upstream_error", `${method}: the page did not answer in ${timeoutMs / 1000}s`));
    }, timeoutMs);
    chrome.debugger.sendCommand({ tabId }, method, params, (result) => {
      if (settled) return;
      settled = true;
      clearTimeout(timer);
      const e = chrome.runtime.lastError;
      if (e) reject(err("upstream_error", `${method}: ${e.message}`));
      else resolve(result || {});
    });
  });
}

export async function attach(tabId) {
  await new Promise((resolve, reject) => {
    const timer = setTimeout(
      () => reject(err("upstream_error", "debugger attach did not answer")),
      COMMAND_TIMEOUT_MS,
    );
    chrome.debugger.attach({ tabId }, CDP_VERSION, () => {
      clearTimeout(timer);
      const e = chrome.runtime.lastError;
      // "Already attached" may be OUR OWN session surviving a worker restart,
      // or another client (DevTools, another extension) holding the tab —
      // Chrome allows only one debugger per target and the message is the same
      // either way. Don't guess: accept it here and prove it below.
      if (e && !/already attached/i.test(e.message)) {
        reject(err("upstream_error", `debugger attach: ${e.message}`));
      } else {
        resolve();
      }
    });
  });

  // Prove the session answers AND that it answers for the tab we asked for.
  //
  // Liveness alone is not enough. A session attached to the WRONG tab answers
  // every command cheerfully and reports that nothing we look for exists — so
  // the caller waits out each deadline hunting elements on a page it was never
  // pointed at, which is indistinguishable from a slow site. (Seen in Arc,
  // where the debugger landed on the app's own page rather than the tab just
  // opened.) Ask the page who it is and check.
  let href = "";
  try {
    const res = await send(
      tabId,
      "Runtime.evaluate",
      { expression: "location.href", returnByValue: true },
      ATTACH_PROBE_MS,
    );
    href = (res && res.result && res.result.value) || "";
  } catch (e) {
    await detach(tabId).catch(() => {});
    throw err(
      "upstream_error",
      "the debugger attached but the tab does not answer it — something else is " +
      "already debugging this tab (DevTools open, or another extension), or this " +
      `browser does not support it. (${e.message})`,
    );
  }

  let expected = "";
  try {
    const tab = await chrome.tabs.get(tabId);
    expected = tab.url || "";
  } catch (e) { /* tab went away; the href check below is then all we have */ }

  if (href && expected && originOf(href) !== originOf(expected)) {
    await detach(tabId).catch(() => {});
    throw err(
      "upstream_error",
      `the debugger attached to the wrong page — asked for ${expected}, but the ` +
      `session answers for ${href}. This browser maps extension tab ids to a ` +
      "different tab than the one that was opened.",
    );
  }
  await send(tabId, "Page.enable");
  await send(tabId, "Runtime.enable");
}

// Detaching a tab that has already gone can leave the callback unfired, and an
// unbounded await here sits in a caller's `finally` — where a hang settles
// nothing, cannot be caught, and wedges the whole serialized queue behind it.
// Releasing the debugger is cleanup: it is never worth blocking on.
const DETACH_TIMEOUT_MS = 2000;

export async function detach(tabId) {
  await Promise.race([
    new Promise((resolve) => chrome.debugger.detach({ tabId }, () => {
      void chrome.runtime.lastError;
      resolve();
    })),
    sleep(DETACH_TIMEOUT_MS),
  ]);
}

/// Evaluate an expression in the page and return its value.
export async function evaluate(tabId, expression) {
  const res = await send(tabId, "Runtime.evaluate", {
    expression,
    returnByValue: true,
    awaitPromise: true,
  });
  if (res.exceptionDetails) {
    throw err("upstream_error", res.exceptionDetails.text || "page evaluation failed");
  }
  return res.result ? res.result.value : undefined;
}

/// Centre point + enabled state of the first node matching `selector`, or null
/// when it isn't in the DOM. Rect is viewport-relative, which is what
/// Input.dispatchMouseEvent wants.
export function elementAt(tabId, selector) {
  const sel = JSON.stringify(selector);
  return evaluate(tabId, `(() => {
    const el = document.querySelector(${sel});
    if (!el) return null;
    const r = el.getBoundingClientRect();
    if (r.width === 0 && r.height === 0) return null;
    return {
      x: r.x + r.width / 2,
      y: r.y + r.height / 2,
      disabled: el.disabled === true || el.getAttribute('aria-disabled') === 'true',
      text: (el.innerText || '').slice(0, 80),
    };
  })()`);
}

/// Poll until `selector` is present (and enabled, when `enabled` is set).
///
/// Evaluation failures are treated as "not yet", not as fatal. A tab that is
/// still navigating has no execution context to evaluate in, so Runtime.evaluate
/// REJECTS rather than returning null — polling a page from the moment it opens
/// would otherwise throw on the first tick and report a missing element on a
/// page that had not started rendering.
export async function waitFor(tabId, selector, { timeoutMs = 15000, enabled = false } = {}) {
  const deadline = Date.now() + timeoutMs;
  let last = null;
  let lastError = null;
  while (Date.now() < deadline) {
    try {
      last = await elementAt(tabId, selector);
      lastError = null;
    } catch (e) {
      last = null;
      lastError = e;
    }
    if (last && (!enabled || !last.disabled)) return last;
    await sleep(250);
  }
  if (last && enabled && last.disabled) {
    throw err("element_gone", `'${selector}' is still disabled after ${Math.round(timeoutMs / 1000)}s`);
  }
  if (lastError) {
    throw err("upstream_error", `could not read the page while waiting for '${selector}': ${lastError.message}`);
  }
  throw err("element_gone", `'${selector}' never appeared`);
}

function easeInOut(t) {
  return t < 0.5 ? 2 * t * t : 1 - Math.pow(-2 * t + 2, 2) / 2;
}

// Where the pointer was left, so the next move starts from it rather than
// jumping from the origin every time.
const lastPoint = new Map(); // tabId -> {x, y}

/// Move the pointer along an eased, slightly bowed path to (x, y).
export async function humanMove(tabId, x, y) {
  const from = lastPoint.get(tabId) || { x: 0, y: 0 };
  const dist = Math.hypot(x - from.x, y - from.y);
  const steps = Math.round(jitter(MOVE_STEPS_MIN, MOVE_STEPS_MAX));
  // One control point off the straight line: a hand arcs, a script doesn't.
  const bow = dist * 0.18;
  const cx = (from.x + x) / 2 + (Math.random() - 0.5) * bow;
  const cy = (from.y + y) / 2 + (Math.random() - 0.5) * bow;
  for (let i = 1; i <= steps; i++) {
    const t = easeInOut(i / steps);
    const mt = 1 - t;
    await send(tabId, "Input.dispatchMouseEvent", {
      type: "mouseMoved",
      x: mt * mt * from.x + 2 * mt * t * cx + t * t * x,
      y: mt * mt * from.y + 2 * mt * t * cy + t * t * y,
    });
    await sleep(jitter(MOVE_STEP_MS_MIN, MOVE_STEP_MS_MAX));
  }
  lastPoint.set(tabId, { x, y });
}

export async function humanClick(tabId, x, y) {
  await humanMove(tabId, x, y);
  await sleep(jitter(40, 130));
  const base = { x, y, button: "left", buttons: 1, clickCount: 1 };
  await send(tabId, "Input.dispatchMouseEvent", { type: "mousePressed", ...base });
  await sleep(jitter(45, 115));
  await send(tabId, "Input.dispatchMouseEvent", { type: "mouseReleased", ...base });
}

const PRINTABLE_ASCII = /^[\x20-\x7E]$/;

function keyDefFor(ch) {
  if (ch === "\n") return { key: "Enter", code: "Enter", keyCode: 13, text: "\r" };
  if (ch === " ") return { key: " ", code: "Space", keyCode: 32, text: " " };
  // A character's code point is its VIRTUAL KEY CODE only for letters and
  // digits, where the two happen to coincide. For punctuation they are
  // unrelated tables, and guessing is not merely wrong but destructive: "."
  // is code point 46, and virtual key 46 is DELETE — typing a full stop ate
  // the character after it. Punctuation carries `text` and no key code, which
  // inserts exactly the character and nothing else.
  const upper = ch.toUpperCase();
  const isLetter = /[a-zA-Z]/.test(ch);
  const isDigit = /[0-9]/.test(ch);
  if (!isLetter && !isDigit) return { key: ch, code: "", keyCode: 0, text: ch };
  return {
    key: ch,
    code: isLetter ? `Key${upper}` : `Digit${ch}`,
    keyCode: upper.charCodeAt(0),
    text: ch,
  };
}

/// Type `text` a character at a time with human-scale gaps.
///
/// Per-character key events are not only less mechanical than one bulk insert —
/// they are what rich-text composers (Draft.js / Lexical, which is what x.com's
/// is) actually listen for. Input.insertText alone is frequently dropped.
/// Characters no single keystroke produces — emoji, CJK — still go through
/// insertText, the same path an IME or a paste takes.
export async function humanType(tabId, text) {
  for (const ch of Array.from(text)) {
    if (PRINTABLE_ASCII.test(ch) || ch === "\n") {
      const def = keyDefFor(ch);
      const common = {
        key: def.key,
        code: def.code,
        windowsVirtualKeyCode: def.keyCode,
        nativeVirtualKeyCode: def.keyCode,
      };
      await send(tabId, "Input.dispatchKeyEvent", { type: "keyDown", text: def.text, ...common });
      await send(tabId, "Input.dispatchKeyEvent", { type: "keyUp", ...common });
    } else {
      await send(tabId, "Input.insertText", { text: ch });
    }
    await sleep(jitter(TYPE_MS_MIN, TYPE_MS_MAX));
  }
}
