<p align="center">
  <img src="logo.svg" width="120" alt="Linggen" />
</p>

<h1 align="center">Linggen</h1>

<p align="center">The Personal AI OS — run AI agents like an OS runs apps.</p>

<p align="center">
  <a href="https://linggen.dev">Website</a> &middot;
  <a href="#documentation">Docs</a>
</p>

Linggen is a personal AI operating system. Add agents and skills by dropping markdown files — no code changes needed. Multiple agents, multiple models, skills, and missions, all orchestrated on your machine.

## Features

- **Skills-first architecture** — add capabilities by creating a `SKILL.md` file. Skills follow the [Agent Skills](https://agentskills.io) open standard, compatible with Claude Code and Codex.
- **Multi-agent management** — create, configure, and switch between agents. Each agent has its own context, skill set, and model preference.
- **Multi-model routing** — connect local models (Ollama), OpenAI API, Claude API, or AWS Bedrock. Define routing policies like `local-first`, `cloud-first`, or custom priority rules.
- **Skills Marketplace** — search, install, and manage community skills from the built-in marketplace UI or via `/skill` chat command.
- **Mission system** — agents self-initiate work when a mission is active, or stay reactive without one.
- **Web UI + TUI** — both interfaces connect to the same backend and share session state in real-time via SSE.
- **Workspace safety** — file operations are scoped to the workspace root. Agent actions are policy-gated per agent.

## Quick Start

### Prerequisites

- Rust toolchain (1.75+)
- Node.js (18+) for the Web UI
- An LLM provider: [Ollama](https://ollama.com) for local models, or an OpenAI-compatible API key

### Build

```bash
# Backend
cargo build

# Web UI
cd ui && npm install && npm run build && cd ..
```

### Configure

Create `linggen.toml` in the project root:

```toml
[[models]]
id = "local"
provider = "ollama"
url = "http://127.0.0.1:11434"
model = "qwen3:32b"
keep_alive = "20m"

# [[models]]
# id = "cloud"
# provider = "openai"
# url = "https://api.openai.com/v1"
# model = "gpt-4o"
# api_key = "sk-..."

[server]
port = 6666

[agent]
max_iters = 100
```

Config search order: `$LINGGEN_CONFIG` env var, `./linggen.toml`, `~/.config/linggen/`, `~/.local/share/linggen/`.

### Run

```bash
# Start TUI + server (default)
cargo run

# Web UI only (http://localhost:6666)
cargo run -- --web

# Dev mode (backend + Vite HMR)
cargo run -- --web --dev   # terminal 1
cd ui && npm run dev       # terminal 2
```

## Adding Agents

Drop a markdown file in `agents/` with YAML frontmatter:

```markdown
---
name: coder
description: Implementation agent that writes and edits code.
tools: ["Read", "Write", "Edit", "Bash", "Glob", "Grep"]
policy: [Patch, Finalize]
---

You are a coding agent. Write clean, tested code.
```

Frontmatter fields: `name`, `description`, `tools`, `model`, `skills`, `work_globs`, `policy`, `idle_prompt`, `idle_interval_secs`.

The agent is available immediately on the next startup — no code changes needed.

## Adding Skills

Create a directory with a `SKILL.md` file:

```
.linggen/skills/my-skill/SKILL.md    # project-scoped
~/.linggen/skills/my-skill/SKILL.md  # global
```

```markdown
---
name: my-skill
description: Does something useful.
allowed-tools: [Bash, Read]
---

Instructions for the agent when this skill is invoked.
```

Invoke skills via `/my-skill` in chat, or the model invokes them automatically based on context.

### Skills Marketplace

Install community skills from the [marketplace](https://github.com/linggen/skills):

- **Web UI**: Settings > Skills > Marketplace tab — search, install, and uninstall with one click.
- **Chat**: `/skill find <query>`, `/skill add <name>`, `/skill delete <name>`, `/skill list`.

Skills are compatible across Linggen, Claude Code, and Codex (shared [Agent Skills](https://agentskills.io) standard).

## Architecture

```
linggen
├── src/
│   ├── main.rs              # CLI entry (clap)
│   ├── config.rs             # TOML config, model/agent spec parsing
│   ├── engine/               # Core agent loop, tool dispatch, action parsing
│   ├── server/               # Axum HTTP server, SSE events, REST API
│   ├── agent_manager/        # Agent lifecycle, run records, model routing
│   ├── skills/               # Skill discovery, loading, marketplace
│   ├── project_store/        # Persistent state (filesystem JSON/JSONL)
│   └── state_fs/             # Session and workspace state
├── agents/                   # Agent spec markdown files
├── ui/                       # React 19 + Vite + Tailwind v4
└── linggen.toml        # Configuration
```

### Tool Contract

Agents interact with the workspace through a fixed set of Claude Code-style tools:

| Tool | Description |
|---|---|
| `Read` | Read file contents (with optional line range) |
| `Write` | Write/overwrite file |
| `Edit` | Exact string replacement within a file |
| `Bash` | Execute shell commands (with timeout) |
| `Glob` | Find files by pattern |
| `Grep` | Search file contents by regex |
| `WebSearch` | Web search via DuckDuckGo |
| `WebFetch` | Fetch and extract content from a URL |
| `Skill` | Invoke a registered skill |
| `AskUser` | Ask the user a question mid-run |
| `Task` | Delegate a task to another agent |
| `capture_screenshot` | Take a screenshot of a URL |
| `lock_paths` / `unlock_paths` | Multi-agent file locking |

### Multi-Agent Runtime

- Agents are spawned by delegation via `Task` — like `fork()`.
- Delegation depth is configurable via `max_delegation_depth` (default 2).
- All actions are policy-gated per agent: `Patch`, `Finalize`, `Delegate` capabilities are declared in frontmatter.
- Run lifecycle is persisted and cancellation cascades through the run tree.

### Real-time Events

The server publishes SSE events consumed by both Web UI and TUI:

`Token`, `Message`, `AgentStatus`, `SubagentSpawned`, `SubagentResult`, `ContentBlockStart`, `ContentBlockUpdate`, `ToolProgress`, `PlanUpdate`, `Outcome`, `TurnComplete`, `ContextUsage`, `QueueUpdated`, `StateUpdated`.

## API Endpoints

| Route | Method | Description |
|---|---|---|
| `/api/chat` | POST | Send a chat message |
| `/api/events` | GET | SSE event stream |
| `/api/agents` | GET | List agents |
| `/api/skills` | GET | List loaded skills |
| `/api/models` | GET | List configured models |
| `/api/projects` | GET/POST/DELETE | Manage projects |
| `/api/sessions` | GET/POST/DELETE | Manage sessions |
| `/api/settings` | GET/POST | Get/update settings |
| `/api/config` | GET/POST | Get/update server config |
| `/api/credentials` | PUT | Update model API keys |
| `/api/marketplace/search` | GET | Search marketplace skills |
| `/api/marketplace/install` | POST | Install a marketplace skill |
| `/api/marketplace/uninstall` | DELETE | Uninstall a marketplace skill |
| `/api/agent-runs` | GET | List agent runs |
| `/api/agent-children` | GET | List child runs (delegation) |
| `/api/agent-context` | GET | Inspect run context |
| `/api/agent-cancel` | POST | Cancel an active run |
| `/api/ask-user-response` | POST | Respond to an AskUser question |
| `/api/plan/approve` | POST | Approve a plan |
| `/api/storage/roots` | GET | List config directories |
| `/api/storage/tree` | GET | Browse directory tree |
| `/api/storage/file` | GET/PUT/DELETE | Read, write, or delete a file |

## Documentation

Detailed design docs live in [`doc/`](doc/):

| Document | Description |
|----------|-------------|
| [Product Spec](doc/product-spec.md) | Vision, OS analogy, product goals, UX surface |
| [Insight](doc/insight.md) | Market positioning, competitive landscape, strategic direction |
| [Agentic Loop](doc/agentic-loop.md) | Kernel runtime loop — iteration, interrupts, cancellation |
| [Agents](doc/agents.md) | Process management — lifecycle, delegation, scheduling |
| [Skills](doc/skills.md) | Dynamic extensions — format, discovery, triggers |
| [Tools](doc/tools.md) | Syscall interface — built-in tools, safety |
| [Plan](doc/plan-spec.md) | Plan mode — research, approval, execution |
| [Chat System](doc/chat-spec.md) | SSE events, message model, rendering, APIs |
| [Models](doc/models.md) | Providers, routing, credentials, auto-fallback |
| [Storage](doc/storage.md) | Filesystem layout, persistent state, data formats |
| [CLI](doc/cli.md) | CLI reference — commands, flags, modes |
| [Code Style](doc/code-style.md) | Code style rules |
| [Log Spec](doc/log-spec.md) | Logging — levels, throttling, output targets |

For more information, visit [linggen.dev](https://linggen.dev).

## License

MIT
