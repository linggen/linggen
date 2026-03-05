---
type: spec
guide: |
  Product specification — describe what the system should do and why.
  Keep it brief. Aim to guide design and implementation, not document code.
  Avoid implementation details like function signatures, variable types, or code snippets.
---

# Storage

Filesystem layout for all persistent state. No database — everything is files.

## Related docs

- `agents.md`: agent lifecycle, delegation.
- `mission-spec.md`: cron-based mission system.
- `agentic-loop.md`: plan persistence, context management.
- `chat-spec.md`: API endpoints that read/write this state.
- `skills.md`: skill discovery paths.

## Root directories

Two-tier layout, aligned with Claude Code's `~/.claude/` + `{repo}/.claude/` convention.

| Directory | Purpose |
|-----------|---------|
| `~/.linggen/` | Global home (override with `$LINGGEN_HOME`) |
| `~/.linggen/projects/{encoded}/` | Per-project state (sessions, runs, missions) |
| `{workspace}/.linggen/` | Workspace-local settings (permissions, future config) |

Project path encoding: `/Users/foo/project` → `-Users-foo-project` (same convention as Claude Code).

## Global state (`~/.linggen/`)

```
~/.linggen/
├── config/
│   └── linggen.runtime.toml    # Runtime config overrides (TOML)
├── logs/
│   └── linggen-{YYYY-MM-DD}.log  # Daily rolling logs (text)
├── plans/
│   └── {slug}.md                     # Plan files (markdown)
├── agents/
│   └── {name}.md                     # Global agent specs (markdown + YAML frontmatter)
├── skills/
│   ├── {name}.md                     # Flat skill files
│   └── {name}/SKILL.md              # Nested skill directories
├── credentials.json                  # API keys for model providers (JSON)
├── ling.pid                          # Daemon PID
└── ling.log                          # Daemon stdout
```

## Per-project state (`~/.linggen/projects/{encoded}/`)

```
~/.linggen/projects/{encoded}/
├── project.json                      # Project metadata (JSON)
├── sessions/
│   └── {session_id}/
│       ├── session.yaml              # Session metadata (YAML)
│       └── messages.jsonl            # Chat messages, append-only (JSONL)
├── runs/
│   └── {run_id}.json                 # Agent run records (JSON)
├── missions/
│   └── {mission_id}/
│       ├── mission.json              # Mission definition (JSON)
│       └── runs.jsonl                # Mission run history (JSONL)
└── memory/
    └── ...                           # Agent memory (managed by memory skill)
```

## Workspace-local state (`{workspace}/.linggen/`)

```
{workspace}/.linggen/
└── permissions.json                  # Tool permission allows (JSON)
```

Same pattern as Claude Code's `{repo}/.claude/settings.local.json`. Lives in the repo, can be gitignored.

## Data formats

### Credentials (`credentials.json`)

```json
{
  "gemini-flash": { "api_key": "AIza..." },
  "groq-llama": { "api_key": "gsk_..." }
}
```

Stored at `~/.linggen/credentials.json`. Keyed by model `id` from `linggen.toml`. Never committed to git. See `models.md` → Credentials.

### Project info (`project.json`)

```json
{ "path": "/abs/path", "name": "project-name", "added_at": 1700000000 }
```

### Tool permissions (`permissions.json`)

```json
{ "tool_allows": ["Write", "Edit"] }
```

Project-scoped tool permission allows, stored at `{workspace}/.linggen/permissions.json`. Created when user selects "Allow all {tool} for this project". See `tools.md` → Tool permission mode.

### Session metadata (`session.yaml`)

```yaml
id: sess-1700000000-abc12345
title: "Fix login bug"
created_at: 1700000000
```

### Chat messages (`messages.jsonl`)

One JSON object per line, append-only.

```json
{ "agent_id": "ling", "from_id": "user", "to_id": "ling", "content": "...", "timestamp": 1700000000, "is_observation": false }
```

### Agent run record (`{run_id}.json`)

```json
{
  "run_id": "run-ling-1700000000-123456",
  "repo_path": "/abs/path",
  "session_id": "sess-...",
  "agent_id": "ling",
  "agent_kind": "main",
  "parent_run_id": null,
  "status": "completed",
  "detail": "chat:structured-loop",
  "started_at": 1700000000,
  "ended_at": 1700000060
}
```

Status values: `running`, `completed`, `failed`, `cancelled`.

### Mission (`mission.json`)

```json
{
  "id": "mission-1700000000-abc12345",
  "schedule": "*/30 * * * *",
  "agent_id": "ling",
  "prompt": "Check CI/CD status and report issues",
  "model": null,
  "enabled": true,
  "created_at": 1700000000
}
```

Multiple missions can be active simultaneously — each in its own `missions/{id}/` directory. See [`mission-spec.md`](mission-spec.md) for full details.

### Mission run history (`runs.jsonl`)

```json
{ "run_id": "run-ling-1700000000-123456", "triggered_at": 1700000000, "status": "completed", "skipped": false }
```

Append-only. Skipped triggers (agent busy) logged with `"skipped": true`.

### Plan (`{slug}.md`)

```markdown
# Plan: Refactor auth module

**Status:** approved
**Origin:** model_managed

- [x] Read existing auth code
  src/auth.rs — understand current session handling
- [~] Implement JWT validation
- [ ] Add tests
- [-] Skipped: migration script
```

Status values: `planned`, `approved`, `executing`, `completed`.
Item markers: `[x]` done, `[~]` in progress, `[ ]` pending, `[-]` skipped.

## Configuration

Search order for `linggen.toml`:
1. `$LINGGEN_CONFIG` env var
2. `./linggen.toml` (working directory)
3. `~/.config/linggen/`
4. `~/.local/share/linggen/`

## Implementation

| Module | Responsibility |
|--------|---------------|
| `paths.rs` | All `~/.linggen/` path constants |
| `project_store/mod.rs` | Projects, missions, agent overrides |
| `project_store/runs.rs` | Run records (CRUD) |
| `project_store/path_encoding.rs` | Path → directory name encoding |
| `state_fs/sessions.rs` | Sessions, chat messages (CRUD) |
| `state_fs/mod.rs` | Workspace-level state files |
| `engine/mod.rs` | Plan file persistence |
| `config.rs` | Config loading/saving |
| `logging.rs` | Log file rotation |

## Safety

- **Path traversal**: Session IDs and state file names reject `..`, `/`, `\\`.
- **Append-only messages**: Chat history uses JSONL — concurrent appends are safe.
- **No overwrites on history**: Missions and runs create new files; never overwrite old ones.
