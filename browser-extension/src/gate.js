// Permission gate — the ONE gate for browser control, whoever the caller is
// (a Linggen session, Claude Code / Cursor via the daemon's /mcp, anything).
// Aligned with Claude in Chrome's model:
//
// - Trust is per-origin. "Always allow" persists in chrome.storage.local
//   until revoked from the popup; "Allow once" grants the origin for this
//   browsing session (chrome.storage.session).
// - A hard floor (payment / credentials / destructive / posting as the user)
//   always prompts, even on a trusted origin, and never offers "Always".
// - The prompt is a small extension window — page JS can't render or click
//   it, and a closed window counts as Deny.

import { progress } from "./progress.js";

const PROMPT_TIMEOUT_MS = 120000;

const FLOOR = [
  ["payment", ["pay", "buy", "purchase", "checkout", "place order", "order now", "subscribe", "add card", "billing"]],
  ["security", ["password", "passkey", "two-factor", "2fa", "security key"]],
  ["destructive", ["delete", "remove", "deactivate", "erase", "uninstall"]],
  ["posting", ["post", "tweet", "send", "reply", "publish", "share"]],
];

/// Whole-word / whole-phrase match on the target's accessible name:
/// "Post" trips the floor, "Postpone" doesn't.
export function hardFloorCategory(name) {
  const words = String(name || "").toLowerCase().split(/[^a-z0-9]+/).filter(Boolean);
  for (const [category, needles] of FLOOR) {
    for (const needle of needles) {
      const nw = needle.split(" ");
      for (let i = 0; i + nw.length <= words.length; i++) {
        if (nw.every((w, j) => words[i + j] === w)) return category;
      }
    }
  }
  return null;
}

async function isTrusted(origin) {
  if (!origin) return false;
  const { trusted_origins = {} } = await chrome.storage.local.get("trusted_origins");
  if (trusted_origins[origin]) return true;
  const { session_origins = {} } = await chrome.storage.session.get("session_origins");
  return !!session_origins[origin];
}

async function grant(origin, always) {
  if (!origin) return;
  const area = always ? chrome.storage.local : chrome.storage.session;
  const key = always ? "trusted_origins" : "session_origins";
  const cur = (await area.get(key))[key] || {};
  cur[origin] = Date.now();
  await area.set({ [key]: cur });
}

// In-flight prompts: id -> { resolve, windowId, timer }.
const pending = new Map();
let promptSeq = 0;

chrome.runtime.onMessage.addListener((msg) => {
  if (msg && msg.t === "gate_decision") {
    const p = pending.get(msg.id);
    if (p) p.resolve(msg.decision);
  }
});

// Closing the prompt window without answering is a Deny.
chrome.windows.onRemoved.addListener((windowId) => {
  for (const p of pending.values()) {
    if (p.windowId === windowId) p.resolve("deny");
  }
});

async function prompt(origin, summary, floor) {
  promptSeq += 1;
  const id = `g${Date.now()}-${promptSeq}`;
  const url = chrome.runtime.getURL(
    `src/confirm/confirm.html?id=${id}` +
    `&origin=${encodeURIComponent(origin || "")}` +
    `&summary=${encodeURIComponent(summary)}` +
    (floor ? `&floor=${encodeURIComponent(floor)}` : "")
  );
  const win = await chrome.windows.create({
    url, type: "popup", width: 420, height: floor ? 320 : 290, focused: true,
  });
  const decision = await new Promise((resolve) => {
    const timer = setTimeout(() => resolve("deny"), PROMPT_TIMEOUT_MS);
    pending.set(id, { resolve, windowId: win.id, timer });
  });
  const p = pending.get(id);
  pending.delete(id);
  if (p) clearTimeout(p.timer);
  await chrome.windows.remove(win.id).catch(() => {});
  return decision;
}

/// Gate one mutating action. Resolves silently when allowed; throws
/// `not_permitted` when the user declines (or the prompt times out).
export async function gateAction({ origin, summary, targetName }) {
  const floor = hardFloorCategory(targetName || "");
  if (!floor && (await isTrusted(origin))) return;
  progress("approval", floor
    ? `Waiting for your OK in the browser: ${summary}`
    : `Waiting for your OK in the browser — allow ${hostOf(origin)}?`);
  const decision = await prompt(origin, summary, floor);
  if (decision !== "always" && decision !== "once") {
    const e = new Error(`the user declined: ${summary}`);
    e.code = "not_permitted";
    throw e;
  }
  progress("approved", "Allowed in the browser");
  // Floor approval covers this single action only — nothing is persisted.
  if (!floor) await grant(origin, decision === "always");
}

function hostOf(origin) {
  try {
    return new URL(origin).host || origin;
  } catch {
    return origin || "this site";
  }
}
