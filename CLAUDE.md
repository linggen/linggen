# CLAUDE.md

## Doc and Spec

Read files under `doc/` and follow them. If you find wrong content in any doc file, confirm with the user.

- `doc/product-spec.md` — vision, OS analogy, product goals, UX surface
- `doc/agentic-loop.md` — kernel: loop, interrupts, PTC, cancellation
- `doc/agent-spec.md` — process management: lifecycle, delegation, scheduling
- `doc/skill-spec.md` — dynamic extensions: format, discovery, triggers
- `doc/tool-spec.md` — syscall interface: built-in tools, safety
- `doc/chat-spec.md` — chat system: events, message model, rendering, APIs
- `doc/models.md` — hardware abstraction: providers, routing
- `doc/storage-spec.md` — filesystem layout: all persistent state, data formats
- `doc/cli.md` — CLI reference
- `doc/code-style.md` — code style rules (flat logic, small files/functions, clean code)
- `doc/session-spec.md` — session/context: creators, effective tools, prompt assembly
- `doc/memory-spec.md` — memory system: extraction, storage, two-tier loading
- `doc/mission-spec.md` — cron mission system
- `doc/plan-spec.md` — plan mode feature
- `doc/log-spec.md` — logging levels, throttling, output targets
- `doc/insight.md` — vision, roadmap, competitive positioning
- `doc/webrtc-spec.md` — WebRTC transport: P2P remote access, signaling, data channels
- `doc/browser-bridge-spec.md` — browser bridge: linggen-browser extension ↔ daemon, logged-in session reads (X), on-demand WS
- `doc/mcp-spec.md` — MCP front door: `/mcp` tool groups (browser/x/memory/agent), plugin + ClawHub distribution, retirement map
- `doc/network-spec.md` — who talks to whom: both daemons, every edge and its transport, where each address comes from
- `doc/mcp-client-spec.md` — ling as an MCP *client*: user-added servers, and ling-mem as the built-in one (designed, not built)
- `doc/room-spec.md` — rooms: community model sharing, credits, auto-dispatch
- `doc/permission-spec.md` — permission system: modes, layers, tool classification, remote trust
- `doc/yinyue-spec.md` — Yinyue companion: avatar, voice, session + memory model
- `doc/yinyue-companion-spec.md` — Yinyue's proactive/interactive layer: senses, heralds, `agent_chat`, ambient life-signs
- `doc/perception-spec.md` — what a resident agent knows without being told: world state, the activity log, and when to speak
- `doc/app-action-spec.md` — app actions as tools: one writer per mutation, phone tool registry, typed cross-device calls, destructive confirms

## Build, Test, Run

Standard `cargo` and `npm` invocations throughout. The non-obvious pairs:

```bash
cargo run                          # Start background daemon + open browser (default)
cargo run -- --web --dev           # Dev mode: API only, proxies static assets to Vite
cargo run -- --root /path/to/proj  # Custom workspace root
```

Full-stack dev = `cargo run -- --web --dev` alongside `cd ui && npm run dev`
(HMR, proxies /api). Production: `cd ui && npm run build`, then `cargo run`
(embeds `ui/dist/` via rust-embed — a release binary serves stale UI until
you rebuild dist).

## Architecture

Linggen is a local-first, multi-agent coding assistant. The binary is
`ling`. Default mode starts a background daemon + opens browser. Explore
`src/` and `ui/src/` directly for layout; the non-derivable anchors:

- The three extension types (skills / agents / missions) share one
  `record.rs` + `registry.rs` shape under `engine/`; their disk loaders all
  live under `extensions/`, not beside their types.
- `ui/src/stores/` (Zustand) is the primary UI state pattern; some Settings
  tabs still use local `useState` + direct `fetch` instead.
- Mission cron scheduling lives in `extensions/missions/scheduler.rs`, not
  under `engine/mission/`.

### Configuration

Config search: `$LINGGEN_CONFIG` → `./linggen.toml` → `~/.config/linggen/` → `~/.local/share/linggen/`.

Key sections: `[[models]]` (LLM providers), `[server]` (port), `[agent]` (max_iters, safety mode, tool_permission_mode), `[logging]`, `[[agents]]` (agent spec references), `[routing]` (model selection policies).

## Code Style

Follow `doc/code-style.md`:
- Prefer guard clauses and early returns over deep nesting
- Keep files and functions small and focused; refactor when complexity grows
- Remove unused code — no compatibility shims or dead feature flags
- Keep async control flow explicit and traceable

## Key Design Patterns

- **Tool names are Claude Code-style**: `Read`, `Write`, `Edit`, `Bash`, `Glob`, `Grep` (capitalized).
- **Workspace-scoped file operations**: all paths are sandboxed to workspace root; parent traversal (`..`) is rejected.
- **Capability = tool list**: no separate policy system. If a session has Write/Edit tools, it can patch. If it has Task, it can delegate. See `session-spec.md`.
- **Real-time events**: server publishes events (`Token`, `Message`, `AgentStatus`, `SubagentSpawned`, `ToolStatus`, `PlanUpdate`, `AppLaunched`, etc.) over WebRTC data channels to the web UI.
- **App skills**: skills with `app` frontmatter section run directly (no model). Launcher types: `web` (static files served at `/apps/{name}/`), `bash` (script execution), `url` (external link). Model can also call `RunApp` tool.
- **Delegation depth**: configurable via `max_delegation_depth` (default 2). Any agent can delegate to any other agent.
- **Model routing**: default model chain with health tracking and auto-fallback on errors/rate limits.
- **Tool permissions**: session-scoped, path-aware permission model with four modes (chat/read/edit/admin) and a hardcoded deny floor for catastrophic commands. See `doc/permission-spec.md`.

When working on a task, read the relevant `doc/*.md` spec files for context — don't read all of them upfront.
