---
type: spec
guide: |
  Product specification — describe what the system should do and why.
  Keep it brief. Aim to guide design and implementation, not document code.
  Avoid implementation details like function signatures, variable types, or code snippets.
---

# WebRTC Transport

The primary transport for linggen. WebRTC data channels carry all chat events bidirectionally between the linggen server and browser clients. Works for both local and remote access using the same code path. SSE is retained as a legacy fallback at `/sse`.

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
- Bidirectional — replace SSE + POST with a single channel.
- Works for 80%+ of network configurations via STUN alone.
- Future path to real-time media (camera video, audio).

### Non-goals

- Full mesh or multi-peer topologies — linggen is always one server, one client per connection.
- Guaranteed 100% connectivity for free users — TURN (relay) is a paid-tier add-on for the ~20% where STUN fails.

## Architecture overview

WebRTC is the primary transport. The engine's event model (`ServerEvent`, `UiSseMessage`) is unchanged. WebRTC is the pipe that carries events between server and client.

```
Engine → ServerEvent → events_tx (broadcast)
                          ├── WebRTC handler (primary, all access modes)
                          └── SSE handler    (legacy, at /sse)
```

### Access modes

| Mode | Web UI served from | Signaling via | Data flows |
|:-----|:-------------------|:--------------|:-----------|
| Local | `localhost:9898` | WHIP endpoint on linggen (`/api/rtc/whip`) | WebRTC data channels (local ICE) |
| Remote | `linggen.dev` (static) | WHIP relay (authenticated, CF Worker or Aliyun) | WebRTC data channels (P2P via STUN) |

In local mode, ICE resolves instantly using local candidates — no STUN needed, negligible overhead. The same WebRTC code path handles both cases.

### Remote access via linggen.dev

For remote access, the browser cannot reach `localhost:9898` to load the UI. Instead, `linggen.dev` serves:

| Path | What it serves | Backing |
|:-----|:---------------|:--------|
| `/` | Static web UI (same build as local) | CF Pages (free) |
| `/login`, `/dashboard` | User account, linked instances | CF Worker + KV |
| `/signaling` | WHIP relay (authenticated) | CF Worker + KV |
| `/connect/{instance}` | Direct connection page | CF Pages (static) |

No chat data or media ever passes through `linggen.dev` — only signaling metadata during connection setup.

### Regional hosts

- `linggen.dev` — Cloudflare (default, global)
- `cn.linggen.dev` — Aliyun (future, for China)

Same account system, same relay protocol. User configures which relay to use based on their region.

### Legacy SSE

SSE remains available at `localhost:9898/sse` for backward compatibility (TUI, older clients, debugging). It will be archived once WebRTC is stable and all clients have migrated.

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
3. **Link linggen instance** — run `ling remote login` on the machine. Opens browser for auth, saves token to `~/.linggen/remote.toml`. Or manually set the token in `linggen.toml`.
4. **Instance registers** with relay — on startup, linggen registers itself (instance name, public key) using the API token. Relay associates instance with user account.
5. **Remote connect** — user logs into `linggen.dev`, dashboard shows linked instances with online/offline status. Click to connect via WebRTC.

### What the relay stores per user

- User ID, email, auth credentials.
- Linked instances: instance ID, name, public key, online status, last seen.
- Account tier (free / pro).

### No account needed for local

Local access (`localhost:9898`) requires no account, no token, no registration. Accounts are only for the remote signaling relay.

## Signaling via WHIP

Linggen uses WHIP (WebRTC-HTTP Ingestion Protocol) for signaling. WHIP reduces signaling to a single HTTP POST — no WebSocket, no polling, no custom protocol.

### How WHIP works

1. Client gathers all ICE candidates (ICE full mode — no trickle).
2. Client sends a single HTTP POST with the complete SDP offer in the body.
3. Server processes the offer, gathers its own ICE candidates, returns the complete SDP answer.
4. Peer connection establishes. Done.

One HTTP round trip. No separate ICE exchange endpoint. No stateful signaling channel.

### Local signaling (WHIP on linggen server)

When the UI is loaded from `localhost:9898`, linggen acts as its own WHIP server.

Endpoint: `POST /api/rtc/whip`

- Request body: SDP offer (application/sdp)
- Response body: SDP answer (application/sdp)
- Response status: 201 Created
- No authentication required (same trust model as current HTTP).

No external infrastructure needed. One endpoint, one round trip.

### Remote signaling (WHIP relay)

When the UI is loaded from `linggen.dev`, the client cannot reach linggen's WHIP endpoint directly (behind NAT). The signaling relay bridges the gap.

All relay requests require authentication via the user's session token (for browser clients) or API token (for linggen instances).

Flow:

1. Linggen server long-polls the relay, waiting for offers:
   `GET https://relay/rooms/{instance_id}/offer`
   `Authorization: Bearer <api_token>`
   (blocks until a client connects)

2. Remote client POSTs its SDP offer to the relay:
   `POST https://relay/rooms/{instance_id}/offer`
   `Authorization: Bearer <session_token>`
   (body = SDP offer)

3. Relay verifies both tokens belong to the same user account. Delivers the offer to linggen's waiting GET.

4. Linggen processes the offer, POSTs the SDP answer back:
   `POST https://relay/rooms/{instance_id}/answer`
   `Authorization: Bearer <api_token>`
   (body = SDP answer)

5. Client's original POST returns with the SDP answer.

Two HTTP requests from each side. No WebSocket. The relay is a stateless HTTP queue with auth checks.

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

**Outbound** (server → client): the same `UiSseMessage` events currently carried by SSE — tokens, messages, activity, content blocks, turn completions. Same JSON format, same `kind`/`phase`/`seq` fields.

### Session isolation

Each session's events flow on a dedicated data channel. No filtering needed — events are routed to the correct channel at the source. This eliminates the broadcast + filter pattern and the associated race conditions on session switch.

When the user switches sessions in the UI, the frontend simply reads from a different data channel. Old session channels stay open (messages are cached). No clear/refetch cycle.

### Skill pages in remote mode

Skill web pages (HTML, CSS, JS) are embedded in the linggen binary via `rust_embed` and served locally at `/apps/{skill}/`. In remote mode, the browser can't reach these URLs directly.

Solution: the main UI fetches skill files via the `http_request` message on the control data channel, then loads them into the skill iframe using blob URLs. Skill iframes communicate with the main UI via `postMessage` (which `chat-bridge.js` already does today). The main UI routes skill messages through its WebRTC connection — skills don't need their own transport.

```
Main UI (remote)
  │
  │ 1. Skill needs /apps/game-table/index.html
  │    → sends http_request on dc:control
  │    → linggen returns file content via dc:control
  │    → main UI creates blob URL, sets as iframe src
  │
  └── <iframe src="blob:...">
        │
        │ 2. Skill sends postMessage to parent (chat, events, etc.)
        │    → parent routes through dc:sess-{id}
        │
        └── skill works normally, no HTTP needed
```

No skill files need to be hosted on `linggen.dev`. Skills are fetched on demand from the linggen server through the existing data channel.

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

1. UI loads from `localhost:9898`.
2. Client creates `RTCPeerConnection` with configured ICE servers.
3. Client creates the `control` data channel.
4. Client generates SDP offer, gathers all ICE candidates (full ICE).
5. Client sends SDP offer via `POST /api/rtc/whip` directly to linggen.
6. Server returns SDP answer. Peer connection establishes.
7. Control channel opens. Client creates session data channels as needed.

### Setup (remote)

1. User logs into `linggen.dev`. Dashboard shows linked instances.
2. User clicks an instance to connect. UI loads from `linggen.dev`.
3. Client creates `RTCPeerConnection` with configured ICE servers (including TURN if Pro account).
4. Client creates the `control` data channel.
5. Client generates SDP offer, gathers all ICE candidates.
6. Client sends SDP offer to `linggen.dev/signaling` (WHIP relay), authenticated with session token.
7. Relay matches to the user's instance, delivers offer to linggen's waiting long-poll.
8. Linggen returns SDP answer via relay. Peer connection establishes.
9. Control channel opens. Client creates session data channels as needed.

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

Linggen instances authenticate with the relay using their API token (set during `ling remote login`). The relay verifies the token on every long-poll request.

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

WebRTC works out of the box on `localhost:9898`. No tokens, no accounts.

### Remote mode

Link a linggen instance to a `linggen.dev` account:

```bash
ling remote login
# Opens browser to linggen.dev for authentication
# On success, saves API token to ~/.linggen/remote.toml
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

## SSE legacy

The SSE transport (`GET /api/events`, `POST /api/chat`) remains active for:

- **TUI clients** — the terminal UI uses SSE via `tui_client.rs`. The TUI subscribes to the same `events_tx` broadcast channel as WebRTC.
- **Fallback** — `SseTransport` on the frontend, activated by `?transport=sse` URL parameter or as default when WebRTC is not selected.

The SSE endpoint and broadcast channel will not be removed — they serve TUI clients and act as a fallback. The Web UI defaults to WebRTC.

## Implementation phases

### Phase 1: Transport abstraction ✅

Extracted `Transport` interface on the frontend. Wrapped existing SSE + fetch in `SseTransport`. Introduced `useTransport` hook. No behavior change — pure refactor.

### Phase 2: Local WebRTC with WHIP ✅

Added WHIP endpoint (`POST /api/rtc/whip`). Integrated str0m (Rust WebRTC library, ICE-lite mode) for peer connection and data channels. Implemented `RtcTransport` on the frontend. All Web UI communication (chat, plan actions, AskUser responses, and all `/api/*` calls via fetch proxy) goes through WebRTC when active. Per-session data channels provide natural message isolation. Event buffering handles the session channel creation race. Async HTTP proxying avoids blocking str0m's event loop.

### Phase 3: linggen.dev + user accounts

Build `linggen.dev`: static UI hosting (CF Pages), user accounts (CF Worker + KV), signaling relay (CF Worker). Implement `ling remote login` CLI command. Instance registration and dashboard with online status.

### Phase 4: Regional relays

Deploy signaling relay + static UI on Aliyun for China access (`cn.linggen.dev`). Configure region-appropriate STUN servers. Same account system — user logs in once, accesses instances from any regional host.

### Phase 5: TURN relay (paid tier)

Deploy a TURN server (coturn). TURN credentials issued by `linggen.dev` API based on account tier. Integrate with payment system for Pro accounts.

### Phase 6: Media tracks with WHEP

Camera capture on the server side. WHEP endpoint (`POST /api/rtc/whep/{stream_id}`) for media subscription. Video rendering in the web UI. Skill API for requesting camera/microphone access.
