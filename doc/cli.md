---
type: spec
guide: |
  Product specification — describe what the system should do and why.
  Keep it brief. Aim to guide design and implementation, not document code.
  Avoid implementation details like function signatures, variable types, or code snippets.
---

# CLI Reference

The `ling` binary — Linggen AI coding agent.

## Related docs

- `doc/tools.md`: tool contract, safety rules.
- `doc/agentic-loop.md`: runtime design, loop behaviour.
- `doc/product-spec.md`: product goals, interaction modes, UX.

## Quick reference

```
ling                              # Start server + TUI (default)
ling --web                        # Web UI only, no TUI
ling -d                           # Run as background daemon
ling --port 8080                  # Custom port
ling --root /path/to/project      # Custom workspace

ling stop                         # Stop background daemon
ling status                       # Show agent status
ling doctor                       # Full health diagnostics

ling install                      # Install/update ling
ling update                       # Update ling
ling init                         # Bootstrap skills
ling skills add/remove/list/search
ling eval                         # Run eval tasks
```

## Subcommands overview

| Command | Purpose | Needs full runtime? |
|:--------|:--------|:--------------------|
| *(none)* | Interactive TUI + embedded server | Yes |
| `stop` | Stop background daemon | No |
| `status` | Show agent server status | No |
| `doctor` | Diagnose installation health | No |
| `eval` | Run eval tasks against agents | Yes |
| `init` | Bulk-install skills from `linggen/skills` | No |
| `install` | Install/update the ling binary | No |
| `update` | Update the ling binary | No |
| `skills` | Manage skills (add/remove/list/search) | No |

"Needs full runtime" means the command initialises tracing, `AgentManager`, database, and skill loading. Lightweight commands only load `Config`.

---

## Default (bare `ling`)

Start the agent server with an interactive TUI.

```
ling [OPTIONS]
```

| Flag | Description |
|:-----|:-----------|
| `--root <PATH>` | Workspace root (default: detect `.git`) |
| `--port <PORT>` | Server port (default: `server.port` from config) |
| `--web` | Web UI only — foreground server, no TUI |
| `-d, --daemon` | Run as background daemon (implies `--web`) |
| `--dev` | Dev mode: proxy static assets to Vite dev server |

When no flags are given, `ling` starts the HTTP server on the configured port with the embedded Web UI, then opens the interactive TUI.

### Daemon mode (`-d`)

Spawns a detached child running `ling --web --port <PORT>`:
- Writes PID to `~/.linggen/ling.pid`.
- Daemon stdout/stderr goes to `~/.linggen/ling.log`.
- Polls TCP connect (up to 3 s) and reports readiness.

## stop

Stop the background daemon.

```
ling stop
```

Reads PID from `~/.linggen/ling.pid`, sends `SIGTERM`; if still running after 500 ms, sends `SIGKILL`. Removes the PID file.

## status

Show agent server status.

```
ling status
```

Prints: version, config path, agent server port + running state, workspace root, model count, agent count.

## doctor

Print a diagnostic checklist of the installation.

```
ling doctor
```

Checks (each prints `[OK]`, `[FAIL]`, or `[INFO]` with ANSI colours):

1. Binary version
2. Config file path
3. Workspace root detection
4. Agent server port reachability (TCP probe, 1 s timeout)
5. Model connectivity (Ollama `/api/tags`, OpenAI `/models`)
7. Skills directories (global + project)
8. Agent definition files count
9. Log directory exists and is writable

---

## eval

Run evaluation tasks against agents.

```
ling eval [OPTIONS]
```

| Flag | Description |
|:-----|:-----------|
| `--filter <SUBSTRING>` | Filter tasks by name |
| `--max-iters <N>` | Override max iterations per task |
| `--timeout <SECS>` | Per-task timeout (default: 300) |
| `--agent <ID>` | Override agent for all tasks |
| `--verbose` | Print agent messages during execution |

Exits with code 1 if any task fails. The `--root` global flag sets the workspace root.

---

## init

Bulk-install all skills from the `linggen/skills` GitHub repository.

```
ling init [OPTIONS]
```

| Flag | Description |
|:-----|:-----------|
| `--global` | Install to `~/.linggen/skills/` (default: project `.linggen/skills/`) |

Downloads the repository as a ZIP, scans for all `SKILL.md` files, and extracts each skill directory.

## install

Install or update the `ling` binary.

```
ling install
```

Fetches `manifest.json` from the latest GitHub releases. Downloads the platform-specific binary and installs it.

## update

Update the `ling` binary to latest.

```
ling update
```

Compares versions and skips if already up to date.

---

## skills

Manage skills (install, remove, list, search).

```
ling skills <SUBCOMMAND>
```

### skills add

```
ling skills add <NAME> [OPTIONS]
```

| Flag | Description |
|:-----|:-----------|
| `--repo <URL>` | GitHub repository (default: `linggen/skills`) |
| `--ref <REF>` | Git ref / branch / tag (default: `main`) |
| `--global` | Install to `~/.linggen/skills/` |
| `--force` | Overwrite existing installation |

### skills remove

```
ling skills remove <NAME> [OPTIONS]
```

| Flag | Description |
|:-----|:-----------|
| `--global` | Remove from global scope |

### skills list

```
ling skills list
```

Scans `~/.linggen/skills/`, `.linggen/skills/`, `~/.claude/skills/`, `~/.codex/skills/` and prints installed skill names with their source.

### skills search

```
ling skills search <QUERY>
```

Searches the Linggen marketplace (and skills.sh fallback) and prints results.

---

## Global flags

These flags can be used with any command:

| Flag | Description |
|:-----|:-----------|
| `--root <PATH>` | Workspace root (default: walk up to find `.git`) |
| `--port <PORT>` | Server port (default: from config) |

---

## State files

| File | Purpose |
|:-----|:--------|
| `~/.linggen/ling.pid` | Agent daemon PID (created by `-d`, removed by `stop`) |
| `~/.linggen/ling.log` | Agent daemon stdout/stderr log |

## Configuration

All commands load config from `linggen.toml` (see `doc/storage.md` for search order). Lightweight commands only need the config file; they do not initialise the full agent runtime.

### Model context_window override

For cloud/remote models where the provider API does not report context size:

```toml
[[models]]
id = "ollama1"
provider = "ollama"
url = "http://127.0.0.1:11434"
model = "qwen3.5:cloud"
context_window = 131072
```

## Health endpoint

The server exposes `GET /api/health` returning `{"ok": true}`. Used by daemon mode for readiness polling.
