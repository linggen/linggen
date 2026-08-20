---
type: spec
reader: Coding agent and users
guide: |
  Product specification — describe what the system should do and why.
  Keep it brief. Aim to guide design and implementation, not document code.
  Avoid implementation details like function signatures, variable types, or code snippets.
---

# Browser Bridge

A local bridge between the linggen daemon and the `linggen-browser` Chrome extension. It lets skills read a user's *logged-in* browser sessions (X first) using the user's own session — no paid platform APIs. Cookies never leave the browser; only parsed results cross the bridge.

This is the contract both sides build against. The daemon side lives in this repo; the client side lives in the separate `linggen-browser` repo.

## Related docs

- `skill-spec.md`: how skills run and call local daemon endpoints.
- `chat-spec.md`: the daemon's HTTP/event surface conventions.
- Reference experience: superx.so (reads a logged-in X session without the paid API). We want its read-from-session mechanism, not its automation suite.

## Model

The extension is a **global bridge**: a host shell plus per-site **modules**. The **X module ships first**; future skills add modules (e.g. LinkedIn) without a new Web Store listing. The extension declares host permissions only for enabled modules' domains — `x.com` only, for now.

Reads are **on-demand**: nothing is harvested in the background. A skill asks → the daemon brokers the request to the extension → the extension reads → returns. When a request arrives, the extension opens a hidden `x.com` tab at the relevant URL, lets the **page** make and sign its own API call, intercepts the response (a `document_start` hook wrapping `fetch`/XHR), normalizes it, and closes the tab (read locus "B"). Because the page issues the request, all signing — `queryId`, `features`, `x-csrf-token`, `x-client-transaction-id` — is x.com's own, so the extension forges nothing and X rotating those can't break the bridge.

```
skill script ──HTTP──> daemon ──WS req──> extension ──hidden x.com tab──> X internal API
                         ^                     │
                         └──── WS res ─────────┘  (parsed results only; cookies stay in browser)
```

## Surfaces

The bridge exposes three endpoints on the daemon (`127.0.0.1:9527`):

| Surface | Endpoint | Who calls it | Purpose |
|:--------|:---------|:-------------|:--------|
| Extension socket | `ws://127.0.0.1:9527/api/bridge/socket` | the extension | the request/response channel |
| Skill call | `POST /api/bridge/call` | skill scripts | broker one read, block until the extension answers |
| Status probe | `GET /api/bridge/status` | skill scripts | is the bridge connected and is module X ready? |

Skills speak only HTTP — they never open a WebSocket. The daemon is the broker.

### Skill call

`POST /api/bridge/call` with `{ module, op, params, timeout_ms? }`. The daemon correlates the call to a connected extension, waits for the response (or `timeout_ms`, default 20000), and returns:

- success → `{ ok: true, data }`
- failure → `{ ok: false, code, message }`

`code` is one of: `no_bridge`, `module_unavailable`, `not_logged_in`, `bad_request`, `rate_limited`, `upstream_error`, `timeout`. Skills treat `no_bridge` / `not_logged_in` / `module_unavailable` as "degrade" (fall back to a paid API or empty), and the rest as transient errors.

### Status probe

`GET /api/bridge/status` → `{ connected, ext_version, modules: [{ id, version, ready }] }`. `connected:false` means no extension is attached. Pulse uses this to decide between the bridge path, the paid-API path, and prompting the user to install the extension.

## Transport

The extension is the WebSocket **client**; the daemon is the server. An MV3 worker cannot run a localhost server, so the extension dials the daemon and holds the socket open. On disconnect the extension reconnects with backoff. At most one bridge connection is active; a second connection supersedes the first.

An open socket does **not** by itself keep the MV3 worker alive. Chrome terminates an idle extension service worker after 30 seconds, and only WebSocket *messages* reset that timer — protocol-level ping/pong is handled below the WebSocket API and doesn't count. Since this socket is silent whenever no skill is calling, the extension sends an application-level keepalive every 20 seconds; the daemon ignores it. Without it the worker dies half a minute after each start and the bridge is only connected for part of every minute.

If the socket is closed when a skill calls, the daemon returns `no_bridge` immediately.

## Frames

All frames are JSON with a `t` (type) discriminator.

**Handshake** — on connect the extension announces itself; the daemon accepts or rejects.

```
ext → daemon   { "t": "hello", "ext_version": "1.0.0", "modules": [{ "id": "x", "version": "1" }] }
daemon → ext   { "t": "ready", "bridge_version": "1" }            // or { "t": "reject", "reason": "..." }
```

**Request / response** — `id` correlates a response to its request.

```
daemon → ext   { "t": "req", "id": "01H...", "module": "x", "op": "search", "params": { "query": "local LLM agents", "max": 15 } }
ext → daemon   { "t": "res", "id": "01H...", "ok": true,  "data": [ ... ] }
ext → daemon   { "t": "res", "id": "01H...", "ok": false, "code": "not_logged_in", "message": "no x.com session" }
```

**Status push (optional)** — the extension may notify of a module state change (e.g. the user logged out of X) so the daemon's `status` answer stays fresh:

```
ext → daemon   { "t": "status", "modules": [{ "id": "x", "ready": false }] }
```

**Keepalive** — sent every 20s while the socket is open. It exists to keep Chrome from suspending the extension's service worker (see Transport), not to probe the link, so the daemon ignores it and answers nothing:

```
ext → daemon   { "t": "ping" }
```

## X module

Module id `x`. Ops mirror the reads Pulse needs today (the tools they replace are noted):

| op | params | replaces | state |
|:---|:-------|:---------|:------|
| `search` | `{ query, max }` | `FetchX` (recent search) | wired |
| `targets` | `{ handles[], per_author, max }` | `FetchXTargets` | wired (batched — see below) |
| `own` | `{ username, max }` | `FetchXOwnPosts` | wired |
| `following` | `{ handle, self, max }` | roster: who I already follow | wired |
| `whotofollow` | `{ exclude[], self, max }` | roster: suggestions | wired |
| `post` | `{ text, reply_to? }` | — (the only write) | wired — see **Posting** |
| `mentions` | `{ max }` | `FetchXMentions` | not wired |
| `user_lookup` | `{ username }` | id + follower-count resolution | not wired |
| `followers` | `{ username, max }` | `FetchXFollowers` | not wired |

**Match on shape, not on op names.** X renames its graphql operations freely, and an op that waits for one by name fails silently when it happens: the capture times out, the reader returns an empty list, and an empty list is indistinguishable from "there was nothing". This has now bitten twice — `SearchTimeline`'s wrapper reshaping (2026-08-18) and `UserTweetsAndReplies` disappearing from the profile page (2026-08-20, which emptied Pulse's already-replied list and resurfaced answered posts). Readers whose op name is unstable accumulate results by JSON shape from whatever the tab fires instead.

**Budget the waiting.** A read is mostly waiting: the bridge paces 3-10s before dispatch, the x module another 4-9s before the tab opens, then the capture runs to 20s and the tab dwells 1-4s. Callers must allow ~45s for an ordinary read (`targets` batches and needs far more); a caller timeout under that fails on pacing alone, and every caller renders a timeout as an authoritative empty.

Each op returns the same normalized item shape Pulse already consumes — `{ source:"x", author, handle, followers, title, text, url, score, likes, reposts, replies, age_hours, created_iso }` — so skill-side scoring is unchanged regardless of whether data came from the bridge or the paid API. `[]` is a valid empty result.

## Posting

`post` is the bridge's one write. The read design — hidden tab, listen, close — is wrong for a write in every particular, so it inverts all of it:

- **Visible.** The tab is active and grouped "Linggen", like Browser Control's. A write must be legible while it happens so the user can interrupt it, never something they discover afterwards.
- **Not prompted, because a person already pressed the button.** The gate exists to ask before an *agent* acts. `post` is only reachable when someone clicks Post on a draft in front of them, and confirming a button they just clicked gates the user rather than the agent. The click is the authorization. What makes that safe is the rule below — no model can call this. **If a post tool is ever exposed to a model, the `posting` floor must come back**, and Browser Control's own floor is untouched: clicking a "Post" button on any site via `control` still always confirms.
- **Driven, not filled.** The composer is typed and clicked through CDP (`Input.dispatchKeyEvent` / `dispatchMouseEvent`), not scripted from the page. Two reasons: x.com's composer is a rich-text editor that drops bulk `insertText`, and CDP events reach the page as `isTrusted`, where page-injected events do not.
- **Read back before committing.** The composer's contents are compared against what was typed, and a mismatch aborts. A dropped character is invisible until it is public, and a post cannot be recalled.
- **Confirmed by x.com, not assumed.** It resolves on x.com's own `CreateTweet` response and returns the new post's id. Nothing upstream may report a post as sent that x.com did not confirm. On any failure the tab is left open, so the user can see the real state of their account.

**No agent may call it.** `post` is deliberately absent from the MCP tool table: only a human pressing a button in a skill page reaches it. That is what keeps "the agent never posts" true while letting the user post without leaving the page. Widening this to agent-initiated posting is a separate decision with its own permission design, not a tool registration.

## Security

- The socket binds to `127.0.0.1` only — never exposed remotely (it is not carried over the WebRTC transport).
- The daemon checks the WebSocket upgrade `Origin` against the published extension id (`chrome-extension://<id>`) and rejects others.
- Session cookies, tokens, and CSRF values stay inside the browser. Only parsed result objects cross the bridge.
- The extension's host permissions are scoped to enabled modules' domains; a module the user hasn't enabled has no host access.

## Pulse integration

1. On a gather that wants X, Pulse calls `GET /api/bridge/status`.
2. `connected` and module `x` `ready` → route X reads through `POST /api/bridge/call`.
3. Not connected → fall back to the existing paid X-API path, or emit empty, and surface a one-time deep link to the Web Store listing so the user can install the extension.

The extension cannot ship through `install.sh` (Web Store gated). Pulse's only job is to probe, route, degrade, and link.

## Out of scope

- DMs and follows. `post` is the only write (see **Posting**); everything else the bridge does is a read.
- Background harvesting / polling. On-demand only.
- Any in-page UI in the extension beyond a small status popup.
- Sites other than X. Added as modules later, same contract.
