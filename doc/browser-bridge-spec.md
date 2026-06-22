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

Reads are **on-demand**: nothing is harvested in the background. A skill asks → the daemon brokers the request to the extension → the extension reads → returns. When a request arrives and no relevant tab is open, the extension opens a hidden tab on demand, runs the read in full page context, and closes it (read locus "B" — gets `x-csrf-token` and the page's `x-client-transaction-id` signing for free).

```
skill script ──HTTP──> daemon ──WS req──> extension ──hidden x.com tab──> X internal API
                         ^                     │
                         └──── WS res ─────────┘  (parsed results only; cookies stay in browser)
```

## Surfaces

The bridge exposes three endpoints on the daemon (`127.0.0.1:9898`):

| Surface | Endpoint | Who calls it | Purpose |
|:--------|:---------|:-------------|:--------|
| Extension socket | `ws://127.0.0.1:9898/api/bridge/socket` | the extension | the request/response channel |
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

The extension is the WebSocket **client**; the daemon is the server. An MV3 worker cannot run a localhost server, so the extension dials the daemon and holds the socket open (an open socket also keeps the MV3 worker alive). On disconnect the extension reconnects with backoff. At most one bridge connection is active; a second connection supersedes the first.

WebSocket-level ping/pong is the keepalive. If the socket is closed when a skill calls, the daemon returns `no_bridge` immediately.

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

## X module

Module id `x`. Ops mirror the reads Pulse needs today (the tools they replace are noted):

| op | params | replaces |
|:---|:-------|:---------|
| `search` | `{ query, max }` | `FetchX` (recent search) |
| `user_tweets` | `{ username, max }` | `FetchXOwnPosts` |
| `mentions` | `{ username, max }` | `FetchXMentions` |
| `user_lookup` | `{ username }` | id + follower-count resolution |
| `targets` | `{ handles[], per_author }` | `FetchXTargets` |
| `followers` | `{ username, max }` | `FetchXFollowers` |

Each op returns the same normalized item shape Pulse already consumes — `{ source:"x", author, handle, followers, title, text, url, score, likes, reposts, replies, age_hours, created_iso }` — so skill-side scoring is unchanged regardless of whether data came from the bridge or the paid API. `[]` is a valid empty result.

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

- Writes (posting, replies, DMs, follows). Read-only by design.
- Background harvesting / polling. On-demand only.
- Any in-page UI in the extension beyond a small status popup.
- Sites other than X. Added as modules later, same contract.
