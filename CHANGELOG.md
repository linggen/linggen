# Changelog

## v0.8.0 (2026-03-24)

WebRTC remote access, mobile UI, and infrastructure improvements.

### Added

- **Remote access via WebRTC** — access your linggen from any device. Run `ling login` to link to your linggen.dev account, then connect from any browser at `linggen.dev/app`. Peer-to-peer data channels carry all chat and API traffic — no VPN or port forwarding needed.
- **`ling login` / `ling logout` / `ling status`** — CLI commands for managing remote access. Browser-based OAuth flow with automatic token exchange; fallback to manual paste.
- **Signaling relay** — lightweight relay on linggen.dev handles WebRTC connection setup. Nonce-based offer/answer exchange via stateless HTTP.
- **Mobile UI** — responsive layout auto-detected on narrow viewports (or via `?mode=mobile`). Full-bleed chat, larger touch targets, iOS safe area support. Right-side drawer for models and skills.
- **Gzip chunked transfer** — large responses (skill files, API data) are gzip-compressed and sent as base64 chunks over data channels. Handles SCTP backpressure correctly.
- **Skills open in-app** — web launcher skills now open in an in-page iframe panel instead of a new browser tab. Works in both local and remote mode.
- **Session project names for missions** — mission sessions now show their project name in the session header, matching the behavior of user sessions.

### Changed

- **Heartbeat interval** — increased from 30s to 5 minutes to reduce relay load. Online threshold set to 10 minutes.
- **Online status via D1** — instance online/offline status is now determined by `updated_at` timestamp in D1 database instead of KV TTL keys. Eliminates KV write quota consumption from heartbeats.
- **JWT sessions** — linggen.dev authentication switched from KV-stored sessions to signed JWT cookies (HMAC-SHA256). Eliminates KV reads on every authenticated request.
- **Settings page mobile layout** — scrollable tab strip, responsive model card grid, reduced padding on small screens.
- **Header compact mode** — shorter title ("Linggen" on mobile), status dot without text label, sparkles button for info drawer.
- **Session delete on mobile** — trash button always visible on touch devices (was hover-only).
- **InfoPanel component** — extracted models + skills cards into shared component used by desktop sidebar and mobile drawer.

### Fixed

- **SSRF bypass** — URL-decode path before validation in WebRTC HTTP proxy (blocks `%2e%2e` traversal).
- **JWT algorithm validation** — verify `alg: HS256` in token header before signature check.
- **Free-tier instance limit** — use `COUNT(*)` query instead of single-row check (prevents bypass via new instance IDs).
- **Token panic** — guard `api_token` length before slicing in `ling status` (no crash on corrupted config).
- **Double reconnect** — guard `handleDisconnect` against firing multiple times from concurrent ICE/connection state changes.
- **Double connect** — guard `doConnect` against concurrent calls (prevents RTCPeerConnection leak).
- **Session channel leak** — `unsubscribeSession` now called on session change in `useTransport` hook.
- **Token lost on write error** — browser response write in `ling login` callback no longer discards the received token if the browser closes early.
- **Relay poll blocking** — `handle_remote_offer` spawned in separate task so the offer poll loop stays responsive.
- **Nonce URL encoding** — relay signaling nonce is now URL-encoded in poll requests.
- **Relay offer missing Content-Type** — added `Content-Type: application/sdp` to relay offer POST.
- **Logout CORS headers** — logout response now includes CORS headers for cross-origin requests.
- **Dead code** — removed unused `split_utf8_safe` function.
- **Stale comments** — updated heartbeat interval comments from "30s" to "5 minutes".

## v0.7.0 (2026-03-11)

Major release with native tool calling, mission system, TUI, permissions, and extensive UI improvements.

### Added

- **Native tool calling** — models use structured function calling (OpenAI, Ollama) instead of JSON-in-text. Default for all providers; falls back gracefully for legacy models.
- **TUI interface** — full terminal UI via ratatui. Default mode runs TUI + embedded server; `--web` for web-only.
- **Mission system** — agents self-initiate work on cron schedules when a mission is active. Idle scheduler prompts agents between user messages.
- **Plan mode** — agents can enter plan mode (`EnterPlanMode`) for research and structured planning before making changes. Plans require user approval via `ExitPlanMode`.
- **File-scoped permissions** — `AcceptEdits` mode, deny rules, and per-project permission persistence.
- **Credential storage** — secure API key management via `/api/credentials` endpoint.
- **Model auto-fallback** — health tracking with automatic fallback to next model in the routing chain on errors or rate limits.
- **AskUser bridge** — agents can ask structured questions mid-run with options and multi-select.
- **Web search & fetch** — `WebSearch` (DuckDuckGo) and `WebFetch` tools for agents.
- **Skills marketplace** — search, install, and manage community skills from the web UI or CLI (`ling skills add/remove/search`).
- **`ling init` command** — scaffolds `~/.linggen/` directory tree, installs default agents, creates config, downloads skills.
- **`ling auth` command** — ChatGPT OAuth authentication (browser and device code flows).
- **Session-scoped SSE** — events are tagged with session ID; clients filter to their own session.
- **Per-session working directory** — `cd` in one session doesn't affect others.
- **SSE reconnect handling** — automatic state resync on reconnect with UI indicator.
- **Context window management** — adaptive compaction with importance-based message pruning.
- **Prompt caching** — stable system prompt prefix cached across iterations.

### Changed

- Config file renamed from `linggen.toml` to `linggen.runtime.toml`.
- Prompt system refactored from hardcoded strings to TOML templates.
- Tool calls render as individual inline widgets (aligned with Claude Code style).
- `ChatPanel.tsx` refactored into focused modules under `chat/` folder.
- `tools.rs` and `app.rs` split into module directories for maintainability.
- Default `supports_tools` changed to `true` even for unrecognized model IDs (prevents fallback to text-based JSON mode).

### Fixed

- SSE session isolation — events no longer leak across sessions.
- Streamed text-only responses no longer disappear after generation.
- Ollama 500 error — use role `"tool"` for tool result messages in native mode.
- Agent context loss on long conversations.
- Glob pattern matching edge cases.
- Queued message display order (now chronological).
- Think tag stripping for models that emit `<think>` blocks.

## v0.1.1 (2025-12-15)

Initial patch release.

## v0.1.0 (2025-12-14)

Initial release — multi-agent engine, web UI, skills system, Ollama and OpenAI providers.
