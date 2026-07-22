---
type: spec
reader: Coding agent and users
guide: |
  Product specification — describe what the system should do and why.
  Keep it brief. Aim to guide design and implementation, not document code.
  Avoid implementation details like function signatures, variable types, or code snippets.
---

# WebRTC Transport

The transport for linggen. WebRTC data channels carry all chat events bidirectionally between the linggen server and browser clients. Works for both local and remote access using the same code path.

## Related docs

- `chat-spec.md`: event model, message types, rendering.
- `session-spec.md`: session lifecycle, creators, isolation.
- `skill-spec.md`: skill communication model (interactive apps use sessions + events).

## Motivation

Linggen is free open-source software that runs on the user's own machine. Users need to access it from anywhere — locally, on LAN, or remotely across the internet. A single transport that works in all scenarios is simpler than maintaining separate transports for local vs remote.

WebRTC provides:

- **P2P connectivity** with built-in NAT traversal (ICE/STUN) — no port forwarding or VPN needed.
- **Per-session data channels** — natural message isolation without broadcast + filter.
- **Bidirectional** — chat messages flow both ways on the same channel (no separate POST).
- **Media tracks** — future path to real-time camera video and audio.
- **Same code path** for local and remote — fewer bugs, one thing to maintain.

The signaling overhead is minimal (a few KB of SDP/ICE candidates), cheap to relay via a lightweight signaling server. Actual data flows directly between peers — no ongoing relay cost.

### Goals

- One transport for local, LAN, and remote access.
- Per-session data channels for natural message isolation.
- Bidirectional — single channel for all communication.
- Works for 80%+ of network configurations via STUN alone.
- Future path to real-time media (camera video, audio).

### Non-goals

- Full mesh or multi-peer topologies — linggen is always one server, one client per connection.
- Guaranteed 100% connectivity for free users — TURN (relay) is a paid-tier add-on for the ~20% where STUN fails.

## Architecture overview

WebRTC is the sole transport. The engine's event model (`ServerEvent`, `UiEvent`) is unchanged. WebRTC is the pipe that carries events between server and client.

```
Engine → ServerEvent → events_tx (broadcast) → WebRTC handler
```

### Access modes

| Mode | Web UI served from | Signaling via | Data flows |
|:-----|:-------------------|:--------------|:-----------|
| Local | `localhost:9527` | WHIP endpoint on linggen (`/api/rtc/whip`) | WebRTC data channels (local ICE) |
| Remote | Bootstrap from `linggen.dev`, full UI via data channel | WHIP relay (authenticated, CF Worker or Aliyun) | WebRTC data channels (P2P via STUN) |

In local mode, ICE resolves instantly using local candidates — no STUN needed, negligible overhead. The same WebRTC code path handles both cases.

### Remote access via linggen.dev

For remote access, the browser cannot reach `localhost:9527`. `linggen.dev` serves a lightweight bootstrap page that establishes the WebRTC connection, then loads the full UI from the linggen server through the data channel.

| Path | What it serves | Backing |
|:-----|:---------------|:--------|
| `/login`, `/dashboard` | User account, linked instances | CF Worker + KV |
| `/signaling` | Signaling relay (authenticated) | CF Worker + KV |
| `/connect/{instance}` | Bootstrap page (WebRTC + UI loading via DC) | CF Worker |

No UI build artifacts are deployed to `linggen.dev`. The linggen server is the single source of truth for its UI. No chat data, media, or UI assets pass through `linggen.dev` — only signaling metadata during connection setup.

### Regional hosts

- `linggen.dev` — Cloudflare (default, global)
- `cn.linggen.dev` — Aliyun (future, for China)

Same account system, same relay protocol. User configures which relay to use based on their region.

## User accounts

Remote access requires a free user account on `linggen.dev`. Accounts provide identity for the signaling relay — the relay needs to know which linggen instance belongs to which user.

### Why accounts (not just instance keys)

- **Dashboard**: login once, see all linked linggen instances, click to connect.
- **Multi-instance**: manage home desktop, office server, etc. from one account.
- **Recovery**: lost instance key? Re-link from the dashboard.
- **Paid tier**: natural place to attach TURN credits, push notifications.
- **Abuse prevention**: rate limiting per user, not per IP.

### Account lifecycle

1. **Sign up** on `linggen.dev` — email + password, or OAuth (GitHub, Google). Free, no payment required.
2. **Get API token** from dashboard — a long-lived token for linking linggen instances.
3. **Link linggen instance** — sign in to linggen.dev from the app (or `ling account login`). The token lands in `~/.linggen/account.toml` and the machine registers itself against `~/.linggen/instance_id`; there is no separate link step and no second credential.
4. **Instance registers** with relay — on startup, linggen registers itself (instance name, public key) using the API token. Relay associates instance with user account.
5. **Remote connect** — user logs into `linggen.dev`, dashboard shows linked instances with online/offline status. Click to connect via WebRTC.

### What the relay stores per user

- User ID, email, auth credentials.
- Linked instances: instance ID, name, public key, online status, last seen.
- Account tier (free / pro).

### No account needed for local

Local access (`localhost:9527`) requires no account, no token, no registration. Accounts are only for the remote signaling relay.

## Signaling via WHIP

Linggen uses WHIP (WebRTC-HTTP Ingestion Protocol) for signaling. WHIP reduces signaling to a single HTTP POST — no WebSocket, no polling, no custom protocol.

### How WHIP works

1. Client gathers all ICE candidates (ICE full mode — no trickle).
2. Client sends a single HTTP POST with the complete SDP offer in the body.
3. Server processes the offer, gathers its own ICE candidates, returns the complete SDP answer.
4. Peer connection establishes. Done.

One HTTP round trip. No separate ICE exchange endpoint. No stateful signaling channel.

### Local signaling (WHIP on linggen server)

When the UI is loaded from `localhost:9527`, linggen acts as its own WHIP server.

Endpoint: `POST /api/rtc/whip`

- Request body: SDP offer (application/sdp)
- Response body: SDP answer (application/sdp)
- Response status: 201 Created
- No authentication required (same trust model as current HTTP).

No external infrastructure needed. One endpoint, one round trip.

### Remote signaling (relay)

When connecting remotely, the client cannot reach linggen's WHIP endpoint directly (behind NAT). The signaling relay on `linggen.dev` bridges the gap. All relay requests are authenticated (session token for browser clients, API token for linggen instances).

Flow:

1. Linggen server polls the relay for incoming offers (`GET /api/signaling/{instance_id}/offer`).
2. Remote client POSTs its SDP offer to the relay, receives a nonce.
3. Linggen picks up the offer, creates a peer connection, POSTs the SDP answer back with the nonce.
4. Client polls for the answer using the nonce until it arrives.

All stateless HTTP — no WebSocket. The relay is a simple nonce-based queue with auth checks.

### Why WHIP over custom signaling

| | Custom signaling | WHIP |
|:--|:-----------------|:-----|
| Local signaling | Multiple endpoints, custom protocol | One POST, standard protocol |
| Remote relay | WebSocket or polling + state management | Stateless HTTP queue |
| ICE exchange | Separate endpoint, trickle-ICE complexity | Bundled in SDP, one round trip |
| Ecosystem compatibility | None | OBS, FFmpeg, GStreamer, any WHIP client |
| Endpoints needed | 3+ | 1 |

Bundling ICE candidates in the SDP (ICE full mode) adds ~100-200ms of gathering time before the POST. For linggen's one-client-to-one-server use case, this is negligible.

## Data channels

Each WebRTC peer connection carries multiple data channels, one per purpose.

### Control channel

Name: `control`. Created by the client on connection setup. Carries session lifecycle commands, system messages, and HTTP-like request/response pairs for API calls and file serving.

Messages on the control channel:

| Direction | Type | Purpose |
|:----------|:-----|:--------|
| Client → Server | `session_create` | Create a new session (like `POST /api/sessions`) |
| Server → Client | `session_created` | Confirms session, returns session_id |
| Client → Server | `session_destroy` | End a session |
| Client → Server | `session_list` | List sessions for a project |
| Server → Client | `session_list_result` | Session list response |
| Client → Server | `http_request` | Proxied HTTP request (for API calls, skill files) |
| Server → Client | `http_response` | Proxied HTTP response |
| Bidirectional | `heartbeat` | Keep-alive, detect disconnection |
| Server → Client | `notification` | Global notifications (mission complete, etc.) |

### Session channels

Name: `sess-{session_id}`. One per active session. Carries all chat events for that session — both inbound (user messages) and outbound (agent events).

**Inbound** (client → server): chat messages, ask-user responses, plan approvals, commands. These replace `POST /api/chat`, `POST /api/ask-user-response`, `POST /api/plan/approve`, etc.

**Outbound** (server → client): `UiEvent` messages — tokens, messages, activity, content blocks, turn completions. Same JSON format, same `kind`/`phase`/`seq` fields.

### Session isolation

Each session's events flow on a dedicated data channel. No filtering needed — events are routed to the correct channel at the source. This eliminates the broadcast + filter pattern and the associated race conditions on session switch.

When the user switches sessions in the UI, the frontend simply reads from a different data channel. Old session channels stay open (messages are cached). No clear/refetch cycle.

### UI entries and view signal

Linggen ships three UI entries, each a separate HTML + JS bundle:

| Entry | HTML | Surface | What it renders |
|:------|:-----|:--------|:----------------|
| `main` | `/` (index.html) | Owner's full UI | Sidebar, chat, info panel, settings, missions |
| `embed` | `/embed` | Skill-iframe chat widget (memory, game-table, mac-shifu) and VS Code extension | Just `<ChatWidget>`, pinned to one session |
| `consumer` | `/consumer` | Remote consumer joining a proxy room | Consumer chat with shared-skills panel |

On connect, the client sends a `set_view_context` control message including a `view` field (`"main" | "embed" | "consumer"`). The server uses this to scope pushes so an embed peer never observes cross-session state.

### Embed isolation

Embed peers are pinned to a single session. The server enforces:

- **Broadcasts**: activity, agent_status, run, ask_user, widget_resolved, session_created events from sessions other than the pinned one are dropped on the embed peer's control channel.
- **page_state snapshots**: `pending_ask_user` and `busy_sessions` are filtered to only the pinned session (not the user's full session set).
- **Global fields skipped**: `all_sessions` and `missions` are not sent to embed peers (they don't render the sidebar/mission list).

A skill iframe therefore only receives events for its own session plus truly global notifications (room_chat, global notifications) — the user's other sessions running in the main page cannot leak into the iframe.

### Remote asset loading

In remote mode, all assets (main UI and skill pages) are fetched from the linggen server through the data channel's HTTP proxy. This covers `/index.html`, `/assets/*` (JS/CSS chunks), and `/apps/*` (skill files). Skill iframes are loaded via blob URLs and communicate with the main UI via `postMessage`. No files need to be hosted on `linggen.dev`.

### Media tracks (future)

WebRTC media tracks carry camera video and audio. These are separate from data channels and use WebRTC's native media pipeline (adaptive bitrate, codec negotiation, low latency).

Media tracks are negotiated via the control channel when a skill requests camera access (e.g., a smart home skill opening a camera feed). The UI renders media tracks in `<video>` elements.

### WHEP for media consumption

WHEP (WebRTC-HTTP Egress Protocol) is the counterpart to WHIP — it lets clients pull media streams. When a skill exposes a camera feed, the UI can use WHEP to subscribe:

`POST /api/rtc/whep/{stream_id}` — client sends SDP offer, server returns SDP answer with media tracks.

Same one-POST pattern as WHIP. Same relay mechanism for remote access.

## Instance identification

Each linggen instance generates a stable instance ID (stored in `~/.linggen/instance_id`). This is the "address" used by the signaling relay to route offers to the correct linggen server.

When linked to a user account, instances are identified by their user-chosen name (e.g., "home-desktop", "office-server") in the dashboard. The underlying instance ID is an implementation detail.

For sharing access without requiring login (e.g., showing a friend your linggen), the instance ID can be presented as:

- A short code (e.g., `LING-ABCD-1234`) for manual entry.
- A QR code for scanning from a phone.
- A shareable link (e.g., `linggen.dev/connect/LING-ABCD-1234`).

Shared access links can optionally require a password set by the instance owner.

## ICE configuration

### STUN servers

STUN servers help peers discover their public IP and port mapping. Linggen ships with default STUN server addresses. Users can configure additional servers for their region.

For users in China where Google STUN servers are unreachable, alternative STUN servers (Tencent, self-hosted, etc.) can be configured.

### TURN relay (paid tier)

TURN relays traffic when direct P2P fails (~20% of network configurations, typically symmetric NAT or strict firewalls). TURN carries all data — chat and media — so it has ongoing bandwidth cost.

- **Free users**: STUN only, ~80% success rate. If P2P fails, guidance to use a chat app (Discord skill) as an alternative.
- **Paid users (Pro)**: TURN relay provided with short-lived credentials per connection. 100% connectivity.

TURN credentials are time-limited and issued by linggen.dev's API based on the user's account tier.

## Connection lifecycle

### Setup (local)

1. UI loads from `localhost:9527`.
2. Client creates `RTCPeerConnection` with configured ICE servers.
3. Client creates the `control` data channel.
4. Client generates SDP offer, gathers all ICE candidates (full ICE).
5. Client sends SDP offer via `POST /api/rtc/whip` directly to linggen.
6. Server returns SDP answer. Peer connection establishes.
7. Control channel opens. Client creates session data channels as needed.

### Setup (remote)

1. User logs into `linggen.dev`. Dashboard shows linked instances.
2. User clicks an instance to connect. Bootstrap page loads from `linggen.dev/connect/{instance}`.
3. Bootstrap creates `RTCPeerConnection` with configured ICE servers (including TURN if Pro account).
4. Bootstrap creates the `control` data channel.
5. Bootstrap generates SDP offer, gathers all ICE candidates.
6. Bootstrap sends SDP offer to relay via `RelaySignaling` (POST offer, poll for answer), authenticated with session token.
7. Relay matches to the user's instance, delivers offer to linggen's waiting long-poll.
8. Linggen returns SDP answer via relay. Peer connection establishes.
9. Control channel opens. Bootstrap fetches the full UI (`/index.html`, `/assets/*`) via `http_request` on the control channel.
10. Full UI loads and takes over. Client creates session data channels as needed.

### Reconnection

If the peer connection drops:

1. Detect disconnection via heartbeat timeout or ICE connection state change.
2. Attempt to re-establish with exponential backoff (1s, 2s, 4s, ... up to 30s).
3. New WHIP exchange on each attempt (new SDP offer/answer).
4. If reconnection succeeds, session data channels are recreated and message history is re-synced from the server.
5. If reconnection fails after a threshold, show guidance (check network, check that linggen is running).

### Concurrent connections

A linggen instance can serve multiple WebRTC peer connections simultaneously (e.g., phone and laptop). Each peer connection has its own set of data channels. Session isolation is maintained — two clients can view different sessions, or observe the same session.

## Security

### Signaling

The signaling relay only carries SDP offers and answers — no user data. All relay requests are authenticated with user tokens. The relay verifies that the client and instance belong to the same user account before delivering signaling messages. Signaling messages expire after a short TTL (e.g., 60 seconds).

### Data channels

WebRTC data channels are encrypted by default (DTLS). No additional encryption layer is needed for chat content. The peer connection is authenticated during the DTLS handshake using fingerprints exchanged via signaling.

### Authentication model

| Mode | Who authenticates | How |
|:-----|:------------------|:----|
| Local | No auth needed | Same trust as current HTTP (localhost) |
| Remote (own account) | User logs into linggen.dev | Session token in WHIP relay requests |
| Remote (shared link) | Visitor | Instance password in WHIP Authorization header |

Linggen instances authenticate with the relay using the account token from `account.toml` (written by any sign-in). The relay verifies the token on every long-poll request.

### Account tiers

| | Free | Pro |
|:--|:-----|:----|
| User account | Yes | Yes |
| Linked instances | 1 | Multiple |
| Signaling relay | Yes | Yes |
| STUN (80% connectivity) | Yes | Yes |
| TURN (100% connectivity) | No | Yes |
| Push notifications | No | Yes |

## Configuration

### Local mode (no config needed)

WebRTC works out of the box on `localhost:9527`. No tokens, no accounts.

### Remote mode

Link a linggen instance to a `linggen.dev` account:

```bash
ling account login
# Opens browser to linggen.dev for authentication
# On success, saves the account token to ~/.linggen/account.toml
# Instance registers with relay automatically on next startup
```

Or configure manually in `linggen.toml`:

```toml
[transport]
# Additional STUN servers (beyond built-in defaults)
stun_servers = []

[transport.signaling]
# URL of the signaling relay
relay_url = "https://linggen.dev/signaling"
# API token from linggen.dev dashboard
api_token = "usr_xxxxxxxx"
# Instance name shown in dashboard
instance_name = "home-desktop"
```

## Implementation phases

### Phase 1: Transport abstraction ✅

Extracted `Transport` interface on the frontend. Introduced `useTransport` hook.

### Phase 2: Local WebRTC with WHIP ✅

Added WHIP endpoint (`POST /api/rtc/whip`). Integrated str0m (Rust WebRTC library, ICE-lite mode) for peer connection and data channels. Implemented `RtcTransport` on the frontend. All Web UI communication (chat, plan actions, AskUser responses, and all `/api/*` calls via fetch proxy) goes through WebRTC when active. Per-session data channels provide natural message isolation. Event buffering handles the session channel creation race. Async HTTP proxying avoids blocking str0m's event loop.

### Phase 3: linggen.dev + user accounts

Build `linggen.dev`: bootstrap connect page, user accounts, signaling relay (all CF Worker + KV). Implement account sign-in + instance registration. Instance registration and dashboard. UI loaded from linggen server via data channel — not hosted on linggen.dev.

### Phase 4: Regional relays

Deploy signaling relay + bootstrap page on Aliyun for China access (`cn.linggen.dev`). Configure region-appropriate STUN servers. Same account system — user logs in once, accesses instances from any regional host.

### Phase 5: TURN relay (paid tier)

Deploy a TURN server (coturn). TURN credentials issued by `linggen.dev` API based on account tier. Integrate with payment system for Pro accounts.

### Phase 6: Media tracks with WHEP

Camera capture on the server side. WHEP endpoint (`POST /api/rtc/whep/{stream_id}`) for media subscription. Video rendering in the web UI. Skill API for requesting camera/microphone access.
