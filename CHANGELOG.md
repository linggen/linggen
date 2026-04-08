# Changelog

## v0.9.3 (2026-04-08)

Server-pushed PageState, TUI removal, auto-scroll rework, and UX polish.

### Added

- **Server-pushed PageState** — server aggregates projects, sessions, models, skills, agents, missions, and permissions into a single message pushed over the WebRTC control channel at 0.5 Hz with a dirty-flag mechanism. Replaces the HTTP polling storm that fired on every agent run.
- **`set_view_context` message** — frontend tells the server which session/project is active, scoping PageState pushes to relevant data only.
- **`busy_sessions` in PageState** — remote clients now see session busy status without needing per-session event channels.
- **Dismiss button on queued messages banner** — manually clear the queue when it gets stuck (e.g. `QueueUpdated` event missed).
- **Markdown links open in new tab** — `target="_blank"` on rendered links so clicking doesn't navigate away and text selection is easier.

### Changed

- **Auto-scroll rework** — replaced distance-threshold detection with scroll-direction detection. Added `distanceFromBottom > 150` guard so layout reflows during streaming no longer falsely detach auto-scroll. Consolidated duplicate scroll tracking from ChatPanel into the single `useAutoScroll` hook.
- **Removed HTTP polling** — initial load fetches for projects, sessions, models, skills, agents, and config are all replaced by PageState delivery on WebRTC connect. Only Ollama status and session tokens remain as HTTP fetches.
- **Removed 5 dead API endpoints** — `agent-children`, `agent-context`, `missions/:id GET`, `missions/:id/sessions`, `builtin-skills/install-all`.
- **SessionModeSelector simplified** — reads mode and zone from store (pushed by PageState) instead of fetching `/api/sessions/permission` on every render.
- **Extracted non-reactive agent tracking** — `agentTracker.ts` singleton replaces 15+ direct Zustand store mutations in `eventDispatcher`.
- **Memoized skill suggestions** in ChatInput (was rebuilt twice per render).
- **Tokens/sec display wired up** — `recordTokenSample` + `recomputeTokenRate` were never called; now functional.

### Removed

- **TUI** — terminal UI (ratatui) and `--tui` flag removed. Linggen is now Web UI only. `ling` starts the daemon and opens the browser; `ling --web` runs the server in foreground.
- **SSE transport** — server-sent events transport removed. All real-time communication uses WebRTC data channels.

### Fixed

- **Auto-scroll fighting** — removed duplicate scroll tracker in ChatPanel that competed with `useAutoScroll` hook.
- **Session mode selector race** — after user switches mode (e.g. admin → read), a 3-second suppress window prevents the next PageState push from overwriting the optimistic UI update.
- **Page flash during streaming** — `floatingUserMsg` effect was re-subscribing on every token.
- **Duplicate React key** in SubagentTreeView.
- **Subagent state leak** — `agentTracker.reset()` called on session switch.
- **Plan message overwrite** — `mutateLast` guard prevents fast-path from overwriting plan messages with streaming tokens.
- **Skill session chat in remote mode** — skill app chat iframe now routes through the relay connect page when accessed via linggen.dev (was loading the landing page instead of the compact chat).
- **Skill session restore** — reopening an existing skill session with no localStorage cache auto-triggers a fresh scan instead of showing an empty dashboard.

## v0.9.1 (2026-03-31)

Simplified run system, daemon mode, ChatGPT default model, and bug fixes.

### Added

- **Background daemon mode** — bare `ling` now spawns a background daemon and opens the Web UI in the browser. Terminal returns immediately. Use `ling --tui` for classic TUI mode.
- **ChatGPT OAuth default** — new installs default to GPT-5.4 via ChatGPT subscription. No API key or local model download needed.
- **Unified working folder** — all tools (Read, Write, Edit, Glob, Grep) resolve relative paths from the agent's cwd, not just Bash. When the agent `cd`s into a git repo, the workspace root, CLAUDE.md, and permissions update automatically.
- **User `! cd` tracking** — `! cd /path` in the Web UI now persists cwd per session, same as agent Bash commands.
- **UI follows cwd changes** — `selectedProjectRoot` updates when the agent changes working folder.

### Changed

- **In-memory run store** — agent run records are no longer persisted to `{run_id}.json` files on disk. Runs are tracked in memory only (for cancellation and status during execution).
- **Removed run history UI** — run picker dropdowns, context display, timeline, pin/unpin removed from ChatPanel and SubagentDrawer.
- **Removed dead code** — `AgentsCard.tsx` (never imported), `timeline.ts`, run context types (`AgentRunSummary`, `AgentRunContextResponse`, etc.).
- **Simplified cancel response** — `POST /api/agent-cancel` returns `{ status: "ok" }` instead of `{ cancelled_run_ids: [...] }`.
- **Font size +1px** — all UI font sizes bumped by 1px for mobile readability.
- **Logo** — shortened to "Linggen", links to linggen.dev.
- **install.sh** — removed `--with-memory` flag and ling-mem install block.

### Fixed

- **Plan reject buttons not disappearing** — `PlanUpdate` events now carry `session_id`, so they're delivered via WebRTC data channels (was `None`, events were lost).
- **Queued messages stuck after cancel** — `cancel_agent_run()` now drains the queue for cancelled agents.
- **Queued messages showing in chat** — queued messages are no longer persisted to `messages.jsonl` at queue time. They're persisted when dequeued, preventing the sync-back from re-adding them.
- **Session header not showing for user sessions** — `fetchSessions` was resetting `activeSessionId` when the session wasn't in the project-filtered list. Now checks `allSessions` before resetting.
- **Working folder in non-git dirs** — `check_working_folder_change()` now uses the cwd as workspace root when no git repo is found (was falling back to `~`).
- **macOS `/tmp` symlink** — cwd is canonicalized before use as workspace root (resolves `/tmp` → `/private/tmp`).

## v0.9.0 (2026-03-30)

Working folder model, per-session engines, WebRTC-first transport, and UX improvements.

### Added

- **Working folder model** — sessions start in HOME mode and auto-detect projects when the agent `cd`s into a git repo. CLAUDE.md, permissions, and git context load dynamically on project entry. Configurable `home_path` in settings.
- **Per-session agent engines** — each session gets its own engine instance. No more lock contention between sessions — game-table and regular chat run truly in parallel.
- **WebRTC-first transport** — Web UI always uses WebRTC (local and remote). Per-session data channels provide natural isolation. SSE retained for TUI only.
- **WebRTC session_id enrichment** — events are tagged with session_id before routing to data channels, preventing cross-session event leaks.
- **ChatGPT token expiry UX** — inline re-login button when ChatGPT OAuth expires. After re-login, session engines are cleared so the fresh token is used immediately.
- **Working folder changed event** — `WorkingFolderChanged` server event emitted when the agent `cd`s. UI header updates reactively.
- **`home_path` config** — configurable default working folder for new sessions (defaults to `~`).
- **Git root detection** — `find_git_root()` walks up from cwd looking for `.git/`. Skips home directory dotfiles repos.

### Changed

- **Flat session storage** — all sessions stored in `~/.linggen/sessions/` (flat directory). No more per-project/mission/skill session directories. Session metadata tracks `cwd`, `project`, `project_name`, `mission_id`.
- **Simplified chat creation** — clicking `+` immediately creates a session. Removed project picker dialog.
- **Removed project management UI** — no more workspace section, project cards, or manual project add/remove in sidebar. Projects are auto-discovered from git repos.
- **Skill search ordering** — community skills from skills.sh and ClawHub are interleaved by relevance instead of sorted by install count.
- **ClawHub ZIP install** — handles root-level SKILL.md (no subdirectory) in ClawHub ZIP archives.
- **Ollama status polling** — only polls when Ollama models are configured, eliminating 404 spam.
- **Auto-scroll** — any upward scroll stops auto-scroll (was 10% threshold). Resumes within 20px of bottom.
- **IME composition** — Enter key during Chinese/Japanese input composition no longer triggers send.
- **Models card scroll** — auto-scrolls to default (starred) model when the model list loads.
- **Session list** — session rows use `<div>` instead of nested `<button>` (fixes React DOM nesting warning).
- **Skill reload** — installing/uninstalling skills clears session engines so new skills are available on next message.
- **install.sh** — post-install output now shows `ling init` as the first step.

### Fixed

- **Session isolation** — WebRTC events no longer leak between sessions. Added session_id enrichment in WebRTC peer handler (was missing, only SSE had it).
- **`emit_outcome_event`** — plan/outcome events now carry session_id (was hardcoded `None`).
- **Compact mode race** — skill app iframe now explicitly fetches workspace state after setting `isSkillSession`, preventing stale API calls.
- **Session engine memory leak** — `remove_session_engine` called on all session deletion paths.
- **TUI session creation** — `get_session_meta` check uses `Ok(Some(_))` instead of `is_ok()` (was always true).
- **`UiEvent.kind` type** — added `'working_folder'` to TypeScript union type.

### Removed

- **`~/.linggen/projects/` session directories** — sessions no longer stored per-project.
- **`session_root` on `EngineConfig`** — removed; all persistence goes through global sessions.
- **`ProjectContext.sessions`** — removed; all session access through `AgentManager.global_sessions`.
- **`ProjectStore::session_store()`** — removed dead code.
- **`missions_sessions_dir()` / `skill_sessions_dir()`** — removed from `paths.rs`.
- **`NewChatDialog` component** — removed project picker dialog from UI.

## v0.8.0 (2026-03-25)

Remote access, mobile UI, Google login, and infrastructure improvements.

### Added

- **Remote access** — access your linggen from any device. Run `ling login` to link to your linggen.dev account, then connect from any browser at `linggen.dev/app`. Peer-to-peer connection — no VPN or port forwarding needed.
- **`ling login` / `ling logout` / `ling status`** — CLI commands for managing remote access. Fully automatic browser-based OAuth flow with token exchange; no manual steps needed.
- **`ling auth login`** — ChatGPT subscription auth. Auto-detects headless/SSH environments and falls back to device code flow (removed `--device` flag).
- **Google login** — sign in to linggen.dev with Google or GitHub. Email-based account matching across providers.
- **Signaling relay** — lightweight relay on linggen.dev handles connection setup. Nonce-based offer/answer exchange via stateless HTTP.
- **Mobile UI** — responsive layout auto-detected on narrow viewports (or via `?mode=mobile`). Full-bleed chat, larger touch targets, iOS safe area support. Right-side drawer for models and skills.
- **Gzip chunked transfer** — large responses (skill files, API data) are gzip-compressed and sent as base64 chunks over data channels. Handles SCTP backpressure correctly.
- **Skills open in-app** — web launcher skills now open in an in-page iframe panel instead of a new browser tab. Works in both local and remote mode.
- **Session project names for missions** — mission sessions now show their project name in the session header, matching the behavior of user sessions.

### Changed

- **`ling login` non-interactive** — uses hostname automatically, no instance name prompt.
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
