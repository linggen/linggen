# CLAUDE.md

This file provides guidance to Claude Code (claude.ai/code) when working with code in this repository.


## Doc and Spec

Read files under `doc/` and follow them. If you find wrong content in any doc file, confirm with the user.

- `doc/product-spec.md` — vision, OS analogy, product goals, UX surface
- `doc/agentic-loop.md` — kernel: loop, interrupts, PTC, cancellation
- `doc/agent-spec.md` — process management: lifecycle, delegation, scheduling
- `doc/skill-spec.md` — dynamic extensions: format, discovery, triggers
- `doc/tool-spec.md` — syscall interface: built-in tools, safety
- `doc/chat-spec.md` — chat system: SSE events, message model, rendering, APIs
- `doc/models.md` — hardware abstraction: providers, routing
- `doc/storage-spec.md` — filesystem layout: all persistent state, data formats
- `doc/cli.md` — CLI reference
- `doc/code-style.md` — code style rules (flat logic, small files/functions, clean code)
- `doc/session-spec.md` — session/context: creators, effective tools, prompt assembly
- `doc/mission-spec.md` — cron mission system
- `doc/plan-spec.md` — plan mode feature
- `doc/log-spec.md` — logging levels, throttling, output targets
- `doc/insight.md` — vision, roadmap, competitive positioning
- `doc/webrtc-spec.md` — WebRTC transport: P2P remote access, signaling, data channels

## Build, Test, Run

### Rust Backend

```bash
cargo build                        # Build
cargo test                         # Run all tests
cargo test check::tests            # Run tests in a specific module
cargo test test_name               # Run a single test by name
cargo run                          # Start TUI + embedded server (default)
cargo run -- --web                 # Web UI only, no TUI
cargo run -- --web --dev           # Dev mode (proxy static assets to Vite)
cargo run -- --root /path/to/proj  # Custom workspace root
```

### Web UI (React 19 + Vite + Tailwind v4)

```bash
cd ui
npm install                        # Install dependencies
npm run dev                        # Dev server (HMR, proxies /api to backend)
npm run build                      # Production build → ui/dist/ (embedded by Rust)
npm run lint                       # ESLint check
npm run lint:fix                   # Auto-fix
```

### Full-Stack Dev

Run both in parallel:
1. `cargo run -- --web --dev` (backend API only)
2. `cd ui && npm run dev` (Vite dev server with HMR)

For production: `cd ui && npm run build`, then `cargo run` (embeds `ui/dist/` via rust-embed).

## Architecture

Linggen is a local-first, multi-agent coding assistant. The binary is `ling`. Default mode starts an HTTP server + TUI; `--web` runs the server only.

### Rust Backend (`src/`)

- **`main.rs`** — CLI entry point (clap). Subcommands: `stop`, `status`, `doctor`, `eval`, `init`, `install`, `update`, `skills`. No subcommand → TUI + server.
- **`config.rs`** — Config loading from `linggen.toml` (TOML). Defines `Config`, `ModelConfig`, `AgentSpec` (parsed from markdown frontmatter).
- **`engine/`** — Core agent execution engine. `mod.rs` is the main loop. `tools.rs` implements all model-facing tools (Read, Write, Edit, Bash, Glob, Grep, capture_screenshot, lock_paths, unlock_paths, Task, WebSearch, WebFetch, Skill, AskUser). `actions.rs` parses JSON actions from model output. `streaming.rs` handles streaming responses. `context.rs` manages token counting and compaction. `permission.rs` enforces tool permissions. `plan.rs` manages plan mode.
- **`server/`** — Axum HTTP server. `chat_api.rs` handles chat/run endpoints + SSE streaming. `projects_api.rs` for project/session CRUD. `workspace_api.rs` serves file tree. `config_api.rs` for runtime config. `mission_scheduler.rs` for cron mission scheduling.
- **`agent_manager/`** — Agent lifecycle, run records, cancellation. `models.rs` handles multi-provider dispatch (Ollama, OpenAI-compatible). `routing.rs` implements model selection policies with fallback chains.
- **`tui/`** — Ratatui terminal UI. `app.rs` is the main TUI state machine. `render.rs` draws the interface. `markdown.rs` renders markdown to terminal spans.
- **`ollama.rs`** / **`openai.rs`** — Provider API clients (streaming and non-streaming).
- **`project_store/`** — Persistent state using filesystem JSON files.
- **`skills/`** — Skill discovery, loading, and marketplace integration.
- **`state_fs/`** — Filesystem-backed session state (`.linggen/sessions/`).
- **`check.rs`** — Bash command safety validation (allowlist, not yet wired up).
- **`eval/`** — Evaluation framework: task runner, grader, report generation.
- **`cli/`** — Standalone CLI commands: `daemon.rs`, `doctor.rs`, `self_update.rs`, `init.rs`, `skills_cmd.rs`.

### Web UI (`ui/src/`)

React 19 + TypeScript + Tailwind CSS v4 + Vite.

- **`App.tsx`** — Root component. Project/session management, SSE event handling, page routing.
- **`components/ChatPanel.tsx`** — Chat interface, message rendering, tool activity display.
- **`components/MissionPage.tsx`** — Mission management (editor, agent config, history, activity tabs).
- **`components/SettingsPage.tsx`** — Settings (models, agents, skills, general).
- **`types.ts`** — Shared TypeScript type definitions.

### Agent Definitions (`agents/`)

Agent specs are markdown files with YAML frontmatter. Adding a `.md` file registers a new agent at startup.

Frontmatter fields: `name`, `description`, `tools`, `model`, `personality`.

Current agents: `ling` (the only agent — adapts to any context via skills).

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
- **SSE events**: server publishes real-time events (`Token`, `Message`, `AgentStatus`, `SubagentSpawned`, `ToolStatus`, `PlanUpdate`, `AppLaunched`, etc.) consumed by the web UI.
- **App skills**: skills with `app` frontmatter section run directly (no model). Launcher types: `web` (static files served at `/apps/{name}/`), `bash` (script execution), `url` (external link). Model can also call `RunApp` tool.
- **Delegation depth**: configurable via `max_delegation_depth` (default 2). Any agent can delegate to any other agent.
- **Model routing**: default model chain with health tracking and auto-fallback on errors/rate limits.
- **Tool permissions**: three modes — `auto` (always allow), `warn` (log destructive ops), `ask` (prompt user via AskUser bridge).

When working on a task, read the relevant `doc/*.md` spec files for context — don't read all of them upfront.
