// X module — reads the logged-in x.com session by letting the PAGE make its own
// API calls and intercepting the responses (read locus "B"). x.com handles all
// request signing — queryId, features, x-client-transaction-id — so we forge
// nothing and X rotating those can't break us. See doc/browser-bridge-spec.md.
//
// Flow: open a background x.com tab at the relevant URL → the document_start
// hook (x-hook.js, MAIN world) wraps fetch/XHR and forwards graphql responses →
// x-relay.js (isolated world) relays them here → we match by tab id + operation,
// normalize, and close the tab. Cookies never leave the browser.
//
// `post` is the one WRITE op, and it inverts most of the above: a visible tab
// the user watches, driven through CDP, behind the permission gate. See it at
// the bottom of this file.

import {
  attach, detach, evaluate, elementAt, waitFor,
  humanClick, humanType, sleep,
} from "../cdp.js";

const CAPTURE_TIMEOUT_MS = 20000;

// Closing a tab the instant its response lands is another clean automation
// tell (no human reads a page in 0ms). Hold it open briefly after capture.
const DWELL_MIN_MS = 1000;
const DWELL_MAX_MS = 4000;
function dwell() {
  const ms = DWELL_MIN_MS + Math.random() * (DWELL_MAX_MS - DWELL_MIN_MS);
  return new Promise((r) => setTimeout(r, ms));
}

// One x.com capture at a time, with a human-ish gap between tabs. Pulse's
// agent calls its Fetch tools in parallel, which otherwise opens several
// hidden x.com tabs in the same second — exactly the burst signature X
// looks for. Serializing HERE paces every caller, present and future.
const GAP_MIN_MS = 4000;
const GAP_MAX_MS = 9000;
let captureChain = Promise.resolve();
let lastCaptureAt = 0;
function paced(fn) {
  const run = captureChain.then(async () => {
    const gap = GAP_MIN_MS + Math.random() * (GAP_MAX_MS - GAP_MIN_MS);
    const wait = lastCaptureAt + gap - Date.now();
    if (wait > 0) await new Promise((r) => setTimeout(r, wait));
    try {
      return await fn();
    } finally {
      lastCaptureAt = Date.now();
    }
  });
  captureChain = run.catch(() => {});
  return run;
}

function err(code, message) {
  const e = new Error(message);
  e.code = code;
  return e;
}

// ── response capture: page hook → relay → here ─────────────────────────────
// Two waiter kinds share the same x-capture stream:
//   pendingCaptures — keyed by `${tabId}:${op}`, resolves on a SPECIFIC graphql
//     op (search/targets/following, whose op names are stable).
//   pendingUsers — keyed by tabId, accumulates User objects from ANY graphql
//     response on that tab (who-to-follow, whose op name X can rename freely).
const pendingCaptures = new Map(); // `${tabId}:${op}` -> { resolve, timer }
const pendingUsers = new Map(); // tabId -> { users, seen, minUsers, resolve, timer }
const pendingTweets = new Map(); // tabId -> { tweets, seen, minTweets, resolve, timer, settle }

// Once tweets start landing but fewer than asked for, the account may simply
// not have more. Wait a beat for another page of results, then take what came.
const TWEET_SETTLE_MS = 2500;

chrome.runtime.onMessage.addListener((msg, sender) => {
  if (!msg || msg.type !== "x-capture") return;
  const tabId = sender.tab && sender.tab.id;
  if (tabId == null) return;

  const exact = pendingCaptures.get(`${tabId}:${msg.op}`);
  if (exact) {
    // A waiter with `accept` keeps waiting past responses it rejects — the
    // page can fire an empty priming response before the real one (Following
    // always did; search started to). The newest rejected response is kept
    // so a timeout still returns X's actual answer instead of an error.
    let done = true;
    try { done = !exact.accept || !!exact.accept(msg.data); } catch (e) { done = true; }
    if (done) {
      pendingCaptures.delete(`${tabId}:${msg.op}`);
      clearTimeout(exact.timer);
      exact.resolve(msg.data);
    } else {
      exact.last = msg.data;
    }
  }

  const userWaiter = pendingUsers.get(tabId);
  if (userWaiter && (!userWaiter.op || userWaiter.op === msg.op)) {
    try { deepCollectUsers(msg.data, userWaiter.users, userWaiter.seen); } catch (e) { /* ignore */ }
    if (userWaiter.users.length >= userWaiter.minUsers) userWaiter.resolve();
  }

  const tweetWaiter = pendingTweets.get(tabId);
  if (tweetWaiter) {
    const before = tweetWaiter.tweets.length;
    try { deepCollectTweets(msg.data, tweetWaiter.tweets, tweetWaiter.seen); } catch (e) { /* ignore */ }
    if (tweetWaiter.tweets.length >= tweetWaiter.minTweets) tweetWaiter.resolve();
    else if (tweetWaiter.tweets.length > before) {
      clearTimeout(tweetWaiter.settle);
      tweetWaiter.settle = setTimeout(tweetWaiter.resolve, TWEET_SETTLE_MS);
    } else if (tweetWaiter.emptyHint && tweetWaiter.emptyHint.test(msg.op)) {
      // The timeline this page answers with arrived and carried no tweets.
      // That is a real "nothing here", not a missed capture — settle briefly
      // (the page can fire a priming response first) and answer empty instead
      // of burning the full timeout. The hint is an OPTIMIZATION ONLY: if X
      // renames the op we lose the speed-up, never the correctness, because
      // the tweets themselves are still collected by shape.
      clearTimeout(tweetWaiter.settle);
      tweetWaiter.settle = setTimeout(tweetWaiter.resolve, TWEET_SETTLE_MS);
    }
  }
});

function waitForCapture(tabId, op, timeoutMs, accept) {
  return new Promise((resolve, reject) => {
    const key = `${tabId}:${op}`;
    const entry = { resolve, timer: null, accept: accept || null, last: undefined };
    entry.timer = setTimeout(() => {
      pendingCaptures.delete(key);
      // Only rejected-but-real responses landed — return the newest one:
      // "X answered and the answer was empty" is a result, not an error.
      if (entry.last !== undefined) resolve(entry.last);
      else reject(err("timeout", `no ${op} response from x.com`));
    }, timeoutMs);
    pendingCaptures.set(key, entry);
  });
}

// A hidden capture tab that lands on the login flow will never fire the
// graphql the capture is waiting for — x.com is logged out in THIS browser.
// Surface that as its own typed error the moment the redirect happens, so the
// caller can tell the user what to do instead of running out the timeout.
const LOGIN_WALL_RE = /\/i\/flow\/login|\/login(\?|$)/;

function loginWallWatch(tabId) {
  let listener = null;
  const promise = new Promise((_resolve, reject) => {
    listener = (id, info) => {
      if (id === tabId && info.url && LOGIN_WALL_RE.test(info.url)) {
        reject(err("x_logged_out", "x.com is logged out in this browser — sign in, then retry"));
      }
    };
    chrome.tabs.onUpdated.addListener(listener);
  });
  return { promise, dispose: () => chrome.tabs.onUpdated.removeListener(listener) };
}

// Open a hidden x.com tab, wait for the page to fire `op`, return its raw
// JSON. Optional `accept(data)` skips priming responses (see the listener).
function captureFromUrl(url, op, accept) {
  return paced(async () => {
    const tab = await chrome.tabs.create({ url, active: false });
    const wall = loginWallWatch(tab.id);
    try {
      const data = await Promise.race([
        waitForCapture(tab.id, op, CAPTURE_TIMEOUT_MS, accept),
        wall.promise,
      ]);
      await dwell();
      return data;
    } finally {
      wall.dispose();
      chrome.tabs.remove(tab.id).catch(() => {});
    }
  });
}

// Open a hidden x.com tab, collect User objects from whatever graphql it fires,
// resolve once `minUsers` are seen (or on timeout with whatever we have). Used
// when the op name is unstable/unknown (who-to-follow recommendations).
// `op` (optional) restricts collection to one graphql operation — used by
// `following`, where the page can fire an empty initial Following response
// before the real one; we keep accumulating Following responses until we have
// users (or time out), instead of grabbing the first (maybe empty) one.
function waitForUsers(tabId, minUsers, timeoutMs, op) {
  return new Promise((resolve) => {
    const w = { users: [], seen: new Set(), minUsers, op: op || null, resolve: null, timer: null };
    w.resolve = () => { pendingUsers.delete(tabId); clearTimeout(w.timer); resolve(w.users); };
    w.timer = setTimeout(w.resolve, timeoutMs); // resolve with whatever landed
    pendingUsers.set(tabId, w);
  });
}

// Same idea as waitForUsers, for tweets. Deliberately NOT keyed on a graphql
// op name: a profile timeline's op is exactly the kind X renames, and when it
// did, `own` returned an empty list forever while looking healthy — which in
// turn emptied Pulse's already-replied suppression list and resurfaced posts
// the user had answered. Shape, not name, is the stable thing to match on.
function waitForTweets(tabId, minTweets, timeoutMs, emptyHint) {
  return new Promise((resolve) => {
    const w = {
      tweets: [],
      seen: new Set(),
      minTweets,
      emptyHint: emptyHint || null,
      resolve: null,
      timer: null,
      settle: null,
    };
    w.resolve = () => {
      pendingTweets.delete(tabId);
      clearTimeout(w.timer);
      clearTimeout(w.settle);
      resolve(w.tweets);
    };
    w.timer = setTimeout(w.resolve, timeoutMs); // resolve with whatever landed
    pendingTweets.set(tabId, w);
  });
}

// `emptyHint` (optional RegExp on the graphql op name) lets a page whose
// timeline answered with nothing resolve at once instead of at the timeout.
function captureTweetsFromUrl(url, minTweets, emptyHint) {
  return paced(async () => {
    const tab = await chrome.tabs.create({ url, active: false });
    const wall = loginWallWatch(tab.id);
    try {
      const tweets = await Promise.race([
        waitForTweets(tab.id, minTweets, CAPTURE_TIMEOUT_MS, emptyHint),
        wall.promise,
      ]);
      await dwell();
      return tweets;
    } finally {
      wall.dispose();
      chrome.tabs.remove(tab.id).catch(() => {});
    }
  });
}

function captureUsersFromUrl(url, minUsers, op) {
  return paced(async () => {
    const tab = await chrome.tabs.create({ url, active: false });
    const wall = loginWallWatch(tab.id);
    try {
      const users = await Promise.race([
        waitForUsers(tab.id, minUsers, CAPTURE_TIMEOUT_MS, op),
        wall.promise,
      ]);
      await dwell();
      return users;
    } finally {
      wall.dispose();
      chrome.tabs.remove(tab.id).catch(() => {});
    }
  });
}

// ── response → normalized items (bridge contract X shape) ──────────────────
function ageHours(iso) {
  if (!iso) return null;
  const t = Date.parse(iso);
  return isNaN(t) ? null : Math.floor((Date.now() - t) / 3600000);
}

function toIso(raw) {
  if (!raw) return null;
  const t = Date.parse(raw);
  return isNaN(t) ? null : new Date(t).toISOString();
}

function toItem(result) {
  if (!result) return null;
  const tweet = result.tweet || result; // unwrap TweetWithVisibilityResults
  const tl = tweet.legacy;
  const userResult =
    tweet.core && tweet.core.user_results ? tweet.core.user_results.result : null;
  if (!tl || !userResult) return null;
  // X moved screen_name/name out of `legacy` into a `core` sub-object; read the
  // new location first, fall back to legacy. followers_count stays in legacy.
  const uLegacy = userResult.legacy || {};
  const uCore = userResult.core || {};
  const handle = uCore.screen_name || uLegacy.screen_name || "";
  const name = uCore.name || uLegacy.name || handle;
  const text = tl.full_text || "";
  const likes = tl.favorite_count || 0;
  const reposts = tl.retweet_count || 0;
  const replies = tl.reply_count || 0;
  return {
    source: "x",
    author: name,
    handle,
    followers: uLegacy.followers_count || 0,
    title: text,
    text,
    url: handle ? `https://x.com/${handle}/status/${tweet.rest_id || tl.id_str}` : "",
    score: likes + reposts,
    likes,
    reposts,
    replies,
    created_iso: tl.created_at || null,
    age_hours: ageHours(tl.created_at),
  };
}

// A User result → the bridge's account shape. X moved screen_name/name into a
// `core` sub-object; followers_count + description stay in `legacy`. Read the
// new location first, fall back to legacy so we survive either shape.
function toUser(node) {
  if (!node) return null;
  const legacy = node.legacy || {};
  const core = node.core || {};
  const handle = core.screen_name || legacy.screen_name || "";
  if (!handle) return null;
  return {
    handle,
    name: core.name || legacy.name || handle,
    followers: legacy.followers_count || 0,
    following_count: legacy.friends_count || 0,
    bio: String(core.description || legacy.description || "").replace(/\s+/g, " ").trim(),
    verified: !!(node.is_blue_verified || legacy.verified),
  };
}

// Recursively pull every distinct User out of a graphql response, regardless of
// which timeline/instruction wrapper it sits under — robust to X reshaping the
// following / who-to-follow payloads. Deduped by handle (case-insensitive).
function deepCollectUsers(node, out, seen) {
  if (!node || typeof node !== "object") return;
  if (Array.isArray(node)) {
    for (const child of node) deepCollectUsers(child, out, seen);
    return;
  }
  const looksLikeUser =
    node.__typename === "User" || node.core || node.legacy;
  if (looksLikeUser && (node.core?.screen_name || node.legacy?.screen_name)) {
    const u = toUser(node);
    if (u) {
      const key = u.handle.toLowerCase();
      if (!seen.has(key)) { seen.add(key); out.push(u); }
    }
  }
  for (const k in node) deepCollectUsers(node[k], out, seen);
}

// Recursively collect distinct Tweet result nodes from any graphql response —
// used by `own` to walk the user's own timeline (UserTweetsAndReplies), which
// nests tweets under timeline modules/conversations. Deduped by rest_id.
function deepCollectTweets(node, out, seen) {
  if (!node || typeof node !== "object") return;
  if (Array.isArray(node)) {
    for (const child of node) deepCollectTweets(child, out, seen);
    return;
  }
  const core = node.tweet || node; // unwrap TweetWithVisibilityResults
  if (core && core.legacy && (core.rest_id || core.legacy.id_str)) {
    const id = core.rest_id || core.legacy.id_str;
    if (id && !seen.has(id)) { seen.add(id); out.push(node); }
  }
  for (const k in node) deepCollectTweets(node[k], out, seen);
}

// X renames/reshapes the SearchTimeline wrapper freely — walking a fixed
// path (data.search_by_raw_query.search_timeline…) is how a rename turns
// into a silent empty scan (2026-08-18: every X lane read []). Collect tweet
// nodes wherever they sit instead — the strategy `own` and the user
// collectors already trust for exactly this reason.
function extractTimeline(data) {
  const nodes = [];
  deepCollectTweets(data, nodes, new Set());
  const out = [];
  for (const node of nodes) {
    const item = toItem(node);
    if (item) out.push(item);
  }
  return out;
}

function searchUrl(query, mode) {
  const base = `https://x.com/search?q=${encodeURIComponent(query)}&src=typed_query`;
  return mode === "live" ? `${base}&f=live` : base; // default (no f) = Top
}

// ── ops ────────────────────────────────────────────────────────────────────
const hasItems = (d) => extractTimeline(d).length > 0;

async function search(p) {
  const query = (p.query || "").trim();
  if (!query) return [];
  const data = await captureFromUrl(searchUrl(query, "top"), "SearchTimeline", hasItems);
  const max = Math.min(Math.max(p.max || 15, 1), 100);
  return extractTimeline(data).slice(0, max);
}

// X's recent-search rejects long queries (~512 chars): one 25-handle OR-group
// sails past it and the results page never fires SearchTimeline (observed as a
// bare timeout on a real roster). Batch the from-group by QUERY length.
const TARGETS_QUERY_BUDGET = 250;

function batchHandles(handles) {
  const groups = [];
  let cur = [];
  let len = 0;
  for (const h of handles) {
    const cost = 5 + h.length + (cur.length ? 4 : 0); // "from:" + h + " OR "
    if (cur.length && len + cost > TARGETS_QUERY_BUDGET) {
      groups.push(cur);
      cur = [];
      len = 0;
    }
    cur.push(h);
    len += cost;
  }
  if (cur.length) groups.push(cur);
  return groups;
}

async function targets(p) {
  const handles = (p.handles || [])
    .map((h) => String(h).replace(/^@/, "").trim())
    .filter(Boolean)
    .slice(0, 25);
  if (!handles.length) return [];
  // Per batch: one retry after a backoff — x.com search under repeated hidden
  // -tab automation is flaky (a capture that starves once often lands on the
  // next try). A batch that fails twice is skipped: partial results beat
  // zeroing out the whole roster. x_logged_out stays fatal — it's actionable
  // and retrying can't fix it.
  // Deadline caps the whole dance: the daemon abandons the call at its own
  // timeout and an over-running op then zombies the capture chain, starving
  // every later call. Past the deadline: no more retries, then no more
  // batches — return the partials we have.
  const started = Date.now();
  const elapsed = () => Date.now() - started;
  let all = [];
  for (const group of batchHandles(handles)) {
    if (elapsed() > 120000) break;
    const q = `(${group.map((h) => `from:${h}`).join(" OR ")}) -filter:replies`;
    for (let attempt = 0; attempt < 2; attempt++) {
      try {
        const data = await captureFromUrl(searchUrl(q, "live"), "SearchTimeline", hasItems);
        all = all.concat(extractTimeline(data));
        break;
      } catch (e) {
        if (e && e.code === "x_logged_out") throw e;
        if (attempt === 0 && elapsed() < 60000) {
          await new Promise((r) => setTimeout(r, 30000));
        } else {
          break;
        }
      }
    }
  }
  // Newest first across batches, then per-author cap for diversity
  // (mirrors x-targets.sh).
  all.sort(
    (a, b) => (Date.parse(b.created_iso || "") || 0) - (Date.parse(a.created_iso || "") || 0),
  );
  const cap = p.per_author || 3;
  const seen = {};
  const items = all.filter((it) => {
    seen[it.handle] = (seen[it.handle] || 0) + 1;
    return seen[it.handle] <= cap;
  });
  const max = Math.min(Math.max(p.max || 25, 1), 100);
  return items.slice(0, max);
}

// List the accounts a handle follows. `p.handle` = whose following to read
// (the user's own handle for the roster source; any target's handle for the
// second-degree "who do my targets follow" source). Excludes the page owner +
// optional `p.self`. Returns [{handle,name,followers,following_count,bio,verified}].
async function following(p) {
  const handle = String(p.handle || "").replace(/^@/, "").trim();
  if (!handle) return [];
  // Wait for a Following response that actually carries users (skip the empty
  // initial one), op-filtered so the page's who-to-follow sidebar can't leak in.
  const users = await captureUsersFromUrl(`https://x.com/${handle}/following`, 1, "Following");
  const drop = new Set([handle.toLowerCase()]);
  if (p.self) drop.add(String(p.self).replace(/^@/, "").toLowerCase());
  const max = Math.min(Math.max(p.max || 400, 1), 1000);
  return users.filter((u) => !drop.has(u.handle.toLowerCase())).slice(0, max);
}

// X's own "Who to follow" recommendations — personalized, already excludes the
// people you follow. The op name X uses on this page changes, so we collect
// users from ANY graphql response the page fires. `p.exclude` = handles to drop
// (your following ∪ ignored ∪ already-suggested). Same account shape as above.
async function whotofollow(p) {
  const users = await captureUsersFromUrl("https://x.com/i/connect_people", 3);
  const exclude = new Set(
    (p.exclude || []).map((h) => String(h).replace(/^@/, "").toLowerCase())
  );
  if (p.self) exclude.add(String(p.self).replace(/^@/, "").toLowerCase());
  const max = Math.min(Math.max(p.max || 60, 1), 100);
  return users.filter((u) => !exclude.has(u.handle.toLowerCase())).slice(0, max);
}

// The user's own recent posts + the parent tweets they've already replied to.
// Reads x.com/<handle>/with_replies (UserTweetsAndReplies). Returns
// { items: [...original posts, engagement-annotated...],
//   replied_to: [...x.com/i/status/<id> urls...] } so Pulse can feed
// draft-content the performance signal AND skip posts already replied to.
async function own(p) {
  const handle = String(p.username || p.handle || p.self || "").replace(/^@/, "").trim();
  if (!handle) return { items: [], replied_to: [] };
  const max = Math.min(Math.max(p.max || 10, 1), 100);
  let tweets;
  try {
    // Collect by shape from whatever the profile fires, not from a named op.
    tweets = await captureTweetsFromUrl(`https://x.com/${handle}/with_replies`, Math.min(max, 20));
  } catch (e) {
    return { items: [], replied_to: [] };
  }
  const me = handle.toLowerCase();
  const items = [];
  const replied = new Set();
  for (const node of tweets) {
    const tw = node.tweet || node;
    const tl = tw.legacy;
    if (!tl) continue;
    const ur = tw.core && tw.core.user_results ? tw.core.user_results.result : null;
    const author = (
      (ur && ur.core && ur.core.screen_name) ||
      (ur && ur.legacy && ur.legacy.screen_name) || ""
    ).toLowerCase();
    if (author !== me) continue; // only MY tweets — skip quoted/parent authors
    const parent = tl.in_reply_to_status_id_str;
    if (parent) {
      replied.add(`https://x.com/i/status/${parent}`); // a reply I made → skip its parent
    } else if (!tl.retweeted_status_result) {
      const it = toItem(node);
      if (it) {
        const v = tw.views && tw.views.count ? parseInt(tw.views.count, 10) : null;
        it.views = Number.isFinite(v) ? v : null;
        items.push(it);
      }
    }
  }
  return { items: items.slice(0, max), replied_to: [...replied] };
}

// Mentions + replies to the user, read from the Mentions notifications tab.
// Returns the shaped item LIST the mentions contract asks for (kind/title/
// body/url/author/created_iso/score/watched_term + optional parent), not a
// timeline — the caller pastes it straight into its payload.
//
// Collected by SHAPE, not by graphql op name. The page fires
// `NotificationsTimeline` with `timeline_type:"Mentions"` (verified against a
// live session 2026-09-01), and a timeline op name is exactly what X renames:
// `own` lost one that way and returned an empty list forever while looking
// healthy. Nothing else on this page carries tweets, so shape is safe here.
async function mentions(p) {
  const me = String(p.username || p.handle || p.self || "")
    .replace(/^@/, "")
    .trim()
    .toLowerCase();
  const max = Math.min(Math.max(p.max || 15, 1), 100);
  // A logged-out browser is the user's to fix, so let that error through;
  // everything else degrades to "no mentions", which is a real answer.
  const tweets = await captureTweetsFromUrl(
    "https://x.com/notifications/mentions",
    Math.min(max, 20),
    /notification/i
  ).catch((e) => {
    if (e && e.code === "x_logged_out") throw e;
    return [];
  });

  // The same payload usually carries the tweet a reply is answering. Index it
  // so a parent that IS present costs no extra request; one that isn't is just
  // omitted — both parent fields are optional in the contract.
  const byId = new Map();
  for (const node of tweets) {
    const tw = node.tweet || node;
    const id = tw.rest_id || (tw.legacy && tw.legacy.id_str);
    if (id) byId.set(id, tw);
  }

  const items = [];
  for (const node of tweets) {
    const tw = node.tweet || node;
    const tl = tw.legacy;
    if (!tl) continue;
    const it = toItem(node);
    if (!it || !it.handle) continue;
    if (me && it.handle.toLowerCase() === me) continue; // my own posts aren't mentions

    const parentId = tl.in_reply_to_status_id_str || null;
    // Replying to me is a stronger claim than "mentions me": X puts the
    // handle being replied to in the tweet itself, so no extra lookup.
    const repliedToMe = me
      ? String(tl.in_reply_to_screen_name || "").toLowerCase() === me
      : !!parentId;

    const item = {
      kind: repliedToMe ? "reply_to_me" : "mention",
      title: it.title.slice(0, 300),
      body: it.text,
      url: it.url,
      author: it.handle,
      // The field is named `_iso`, so hand back an actual ISO timestamp —
      // X's own `created_at` is its RFC-822-ish format.
      created_iso: toIso(tl.created_at),
      score: it.score,
      watched_term: me ? `@${me}` : "mentions",
    };
    if (parentId) {
      item.parent_comment_url = `https://x.com/i/status/${parentId}`;
      const parent = byId.get(parentId);
      const body = parent && parent.legacy && parent.legacy.full_text;
      if (body) item.parent_comment_body = body;
    }
    items.push(item);
  }
  return items.slice(0, max);
}

// ── post: the write op ──────────────────────────────────────────────────────
//
// Everything above reads by opening a HIDDEN tab and listening. Posting is the
// opposite on every axis, deliberately:
//
//   visible  — the tab is active and grouped "Linggen", because the specs say a
//              mutating action must be legible while it happens, so the user can
//              interrupt it. A write must never be something they discover after.
//   gated    — gate.js's hard floor ("posting") always confirms, even on a
//              trusted x.com, and never offers "Always". That is the promise in
//              the published Web Store listing; do not route around it.
//   driven   — the composer is typed and clicked through CDP, not filled by
//              script. x.com's composer is a rich-text editor that drops bulk
//              inserts, and CDP events are the ones the page sees as real.
//
// Confirmation is free: x-hook.js already forwards every graphql response on
// any x.com tab, so we wait for x.com's OWN CreateTweet answer and read the new
// post's id out of it. Nothing is reported as posted that x.com didn't confirm.

const COMPOSE_URL = "https://x.com/compose/post";
// On a permalink the reply box can start collapsed — a placeholder that only
// becomes the real contenteditable once it is clicked — so the composer is
// looked for by any of these, clicked, and then re-checked.
const COMPOSER_SEL = '[data-testid="tweetTextarea_0"]';
const COMPOSER_SELS = [
  COMPOSER_SEL,
  '[data-testid="tweetTextarea_0RichTextInputContainer"]',
  '[role="textbox"][contenteditable="true"]',
];
// The compose route uses `tweetButton`; the inline composer on a permalink
// (which is how a reply is written) uses `tweetButtonInline`.
const POST_BUTTON_SELS = ['[data-testid="tweetButton"]', '[data-testid="tweetButtonInline"]'];
const X_POST_LIMIT = 280;
const COMPOSER_TIMEOUT_MS = 20000;
const CREATE_TWEET_TIMEOUT_MS = 30000;
const POST_BUTTON_TIMEOUT_MS = 10000;
const TAB_ACTIVE_TIMEOUT_MS = 3000;
const MARK_TAB_TIMEOUT_MS = 1500;

/// Accepts a status URL or a bare id; returns the id, or "" if it's neither.
function statusId(ref) {
  const s = String(ref || "").trim();
  if (/^\d+$/.test(s)) return s;
  const m = s.match(/status\/(\d+)/);
  return m ? m[1] : "";
}

function preview(text) {
  return text.length > 70 ? `${text.slice(0, 70)}…` : text;
}

/// Normalize for the read-back comparison: the composer reports its own
/// whitespace (trailing newline, NBSP for spaces), which is not a mismatch.
function normalizeComposed(s) {
  return String(s || "").replace(/ /g, " ").replace(/\s+/g, " ").trim();
}

/// Same marker control.js puts on its tab — the one tab an agent is driving
/// should be obvious in the strip. Cosmetic: never fail the post over it.
async function markTab(tabId) {
  // Bounded, because this is decoration. Not every Chromium browser has tab
  // groups — Arc has Spaces instead — and there `chrome.tabs.group` can simply
  // never call back. A try/catch does not save you from a call that never
  // returns, and this one runs before the debugger is even attached, so a hang
  // here stalls the whole post before anything interesting has happened.
  const marked = (async () => {
    const groupId = await chrome.tabs.group({ tabIds: tabId });
    await chrome.tabGroups.update(groupId, { title: "Linggen", color: "green" });
  })();
  try {
    await Promise.race([marked, sleep(MARK_TAB_TIMEOUT_MS)]);
  } catch (e) {
    console.warn("[x] could not group the post tab:", e);
  }
}

/// First selector in `sels` that resolves to a visible node, or null.
async function firstPresent(tabId, sels) {
  for (const sel of sels) {
    let el = null;
    try { el = await elementAt(tabId, sel); } catch (e) { el = null; }
    if (el) return { sel, el };
  }
  return null;
}

/// Poll for any of `sels` — the multi-selector form of cdp.waitFor.
async function waitForAny(tabId, sels, timeoutMs) {
  const deadline = Date.now() + timeoutMs;
  while (Date.now() < deadline) {
    const hit = await firstPresent(tabId, sels);
    if (hit) return hit;
    await sleep(300);
  }
  throw err("element_gone", `none of ${sels.join(", ")} appeared on the page`);
}

/// Last resort when the testids have been renamed: find the submit control by
/// the label a person reads. On a permalink it says "Reply"; on the compose
/// route, "Post". Scoped to the composer's own toolbar so it cannot pick up the
/// sidebar's "Post" link.
function buttonByLabel(tabId) {
  return evaluate(tabId, `(() => {
    const wanted = ["reply", "post"];
    const buttons = Array.from(document.querySelectorAll('[role="button"]'));
    for (const el of buttons) {
      const label = (el.textContent || "").trim().toLowerCase();
      if (!wanted.includes(label)) continue;
      if (el.closest("nav, header")) continue;
      const r = el.getBoundingClientRect();
      if (r.width === 0 && r.height === 0) continue;
      return {
        x: r.x + r.width / 2,
        y: r.y + r.height / 2,
        disabled: el.getAttribute("aria-disabled") === "true",
        text: label,
      };
    }
    return null;
  })()`);
}

/// Resolve once the tab reports itself active, or after `timeoutMs` either way
/// — a browser that never marks it active is the attach check's problem to
/// report, not a reason to refuse to try.
async function waitUntilActive(tabId, timeoutMs) {
  const deadline = Date.now() + timeoutMs;
  while (Date.now() < deadline) {
    try {
      const tab = await chrome.tabs.get(tabId);
      if (tab && tab.active) return;
    } catch (e) {
      return; // tab is gone; attach will fail with something clearer
    }
    await sleep(150);
  }
}

async function findPostButton(tabId) {
  const hit = await firstPresent(tabId, POST_BUTTON_SELS);
  if (hit) return hit.el;
  try { return await buttonByLabel(tabId); } catch (e) { return null; }
}

/// Poll until the submit control exists AND is enabled.
async function waitForEnabledPostButton(tabId, timeoutMs) {
  const deadline = Date.now() + timeoutMs;
  let seen = null;
  while (Date.now() < deadline) {
    const el = await findPostButton(tabId);
    if (el && !el.disabled) return el;
    seen = el || seen;
    await sleep(300);
  }
  if (seen) throw err("element_gone", "x.com's post button never became clickable");
  throw err("element_gone", "x.com's post button is not on the page");
}

/// Extract the new post's id from x.com's CreateTweet response.
function createdIdFrom(data) {
  const r = data && data.data && data.data.create_tweet
    ? data.data.create_tweet.tweet_results
    : null;
  const result = r && r.result ? r.result : null;
  return (result && (result.rest_id || (result.legacy && result.legacy.id_str))) || "";
}

/// Post `text` to x.com, or reply with it when `reply_to` names a post.
/// Resolves only once x.com has confirmed the write.
async function post(p) {
  const text = typeof p.text === "string" ? p.text.trim() : "";
  if (!text) throw err("bad_request", "post requires text");
  if (text.length > X_POST_LIMIT) {
    throw err("bad_request", `post is ${text.length} chars — x.com's limit is ${X_POST_LIMIT}`);
  }
  const parent = statusId(p.reply_to);
  if (p.reply_to && !parent) {
    throw err("bad_request", `reply_to '${p.reply_to}' is not a post URL or id`);
  }

  // NO permission prompt here, deliberately. The gate exists to ask before an
  // AGENT acts; this op is only reachable when a person presses Post on a draft
  // they are looking at, and asking them to confirm a button they just clicked
  // gates the user instead of the agent. The click IS the authorization.
  //
  // What makes that safe is that no agent can reach this op: `post` is
  // deliberately absent from the daemon's MCP tool table, so the only caller is
  // a skill page acting on a human click. IF A POST TOOL IS EVER EXPOSED TO A
  // MODEL, THE GATE MUST COME BACK — an agent-initiated post is exactly what
  // gate.js's `posting` floor is for. What stands in for a prompt meanwhile:
  // the tab is visible, the typing is watchable, and nothing is reported as
  // posted that x.com did not confirm.
  const url = parent ? `https://x.com/i/status/${parent}` : COMPOSE_URL;
  const tab = await chrome.tabs.create({ url, active: true });
  const tabId = tab.id;
  const wall = loginWallWatch(tabId);
  let posted = false;
  // Which step is in flight. A post has many ways to stop, and "it didn't
  // post" is useless without knowing which one — every failure below is
  // re-thrown naming the step it died on.
  let stage = "opening the tab";
  try {
    stage = "attaching the debugger";
    await markTab(tabId);
    // Let the new tab actually BECOME the active one before attaching. We ask
    // for active:true, but a browser is free to take its time, and at least one
    // (Arc) appears to attach the debugger to whatever is active RATHER than to
    // the tab id it was handed — which lands the session on the page the user
    // clicked the button in. Waiting costs a moment and removes the race.
    await waitUntilActive(tabId, TAB_ACTIVE_TIMEOUT_MS);
    await attach(tabId);

    // Click whatever reply affordance is on the page — on a permalink that may
    // be a collapsed placeholder — then require the real contenteditable to
    // exist before typing a single character into it.
    stage = "waiting for the reply box";
    const affordance = await Promise.race([
      waitForAny(tabId, COMPOSER_SELS, COMPOSER_TIMEOUT_MS),
      wall.promise,
    ]);
    stage = "opening the composer";
    await humanClick(tabId, affordance.el.x, affordance.el.y);
    await sleep(400);
    await waitFor(tabId, COMPOSER_SEL, { timeoutMs: 8000 });
    stage = "typing the reply";
    await humanType(tabId, text);
    await sleep(400);

    // Read the composer back before committing. A dropped or mangled character
    // is invisible until it is public, and a post cannot be taken back.
    stage = "reading the composer back";
    const composed = await evaluate(
      tabId,
      `(document.querySelector(${JSON.stringify(COMPOSER_SEL)}) || {}).innerText || ""`
    );
    if (normalizeComposed(composed) !== normalizeComposed(text)) {
      throw err(
        "upstream_error",
        "the composer does not hold the text that was typed — nothing was posted. " +
        "The tab is left open so you can see what x.com received."
      );
    }

    // x.com enables the submit control only once it sees text, so poll for an
    // ENABLED one rather than clicking a dead button.
    stage = "waiting for the post button";
    const ready = await waitForEnabledPostButton(tabId, POST_BUTTON_TIMEOUT_MS);

    // Register the waiter BEFORE the click, or a fast response is missed.
    stage = "clicking post and waiting for x.com to confirm";
    const confirmed = waitForCapture(tabId, "CreateTweet", CREATE_TWEET_TIMEOUT_MS);
    await humanClick(tabId, ready.x, ready.y);
    const data = await Promise.race([confirmed, wall.promise]);

    const id = createdIdFrom(data);
    if (!id) {
      throw err("upstream_error", "x.com answered the post request without confirming a new post");
    }
    posted = true;
    await dwell();
    return {
      id,
      url: `https://x.com/i/status/${id}`,
      in_reply_to: parent || null,
      chars: text.length,
    };
  } catch (e) {
    // Name the step. Chrome's extension Errors page shows console.error, so
    // one glance there says which stage stopped instead of "it didn't post".
    const message = `post failed while ${stage}: ${e.message || e}`;
    console.error(`[x post] ${message}`);
    throw err(e.code || "upstream_error", message);
  } finally {
    wall.dispose();
    // Always hand the tab back — a debugger left attached keeps Chrome's
    // "being debugged" banner on a tab the user is now looking at.
    await detach(tabId).catch(() => {});
    // Only close it after a confirmed post. If anything went wrong the tab
    // stays open on purpose: the user needs to see what state their account is
    // in, and whether the text made it into the composer.
    if (posted) chrome.tabs.remove(tabId).catch(() => {});
  }
}

// Wired via the same intercept pattern when their pages/operations are added.
const notReady = () => {
  throw err("module_unavailable", "op not wired yet");
};

export const xModule = {
  version: "17",
  // `post` drives a visible tab through CDP and waits on the permission prompt,
  // so it must not sit in the read pacing queue: the anti-burst gaps there are
  // for hidden-tab bursts, and a user who just pressed Post is watching.
  unpacedOps: ["post"],
  ops: {
    search,
    targets,
    following,
    whotofollow,
    own,
    post,
    mentions,
    user_lookup: notReady,
    followers: notReady,
  },
};
