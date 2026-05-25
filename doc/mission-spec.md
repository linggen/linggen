---
type: spec
reader: Coding agent and users
guide: |
  Product specification — describe what the system should do and why.
  Keep it brief. Aim to guide design and implementation, not document code.
  Avoid implementation details like function signatures, variable types, or code snippets.
---

# Mission System

Scheduled background agent work. **Missions are a first-class linggen subsystem**, parallel to skills — both are markdown-frontmatter artifacts the engine discovers, loads, and runs. A mission is the headless, scheduled variant: cron-triggered, no human in the loop.

A project or global config can have **multiple active missions** — like a crontab with multiple entries, each an independent task.

## Related docs

- `skill-spec.md`: skill format — missions mirror this shape.
- `agent-spec.md`: agent types, lifecycle, delegation.
- `permission-spec.md`: path-scoped modes and rule evaluation.
- `product-spec.md`: mission system overview, OS analogy.
- `storage-spec.md`: mission JSON format, filesystem layout.

## Mental model

Two sibling subsystems in the linggen engine, discovered and loaded the same way:

**Skill = capability provider.** User-invocable, can ask questions, may render UI, registers capability bindings (`provides:` + `implements:`).

**Mission = headless scheduled task.** Cron-triggered, never asks the user. Consumes tools and capabilities that skills register. No interactive channel — no `AskUser`, no `EnterPlanMode`, no in-mission Web UI.

A mission looks like a `SKILL.md` with a `schedule:` field and none of the interactive affordances. It uses capability tools (like `Memory_*`) that installed skills have registered. It can also delegate to another skill via the `Skill` tool when needed, but that's the exception — typically missions just use tools directly.

## File layout

Missions live under `~/.linggen/missions/` and mirror the skill directory shape:

```
~/.linggen/missions/dream/
├── mission.md         # frontmatter + agent prompt (body)
├── scripts/           # optional — entry scripts, helpers
│   └── collect.sh
├── assets/            # optional — static files
└── runs.jsonl         # run history
```

The mission name is the directory name. One mission per directory. Run history is kept alongside the definition — delete the directory, the mission and its history are gone.

## Frontmatter

```yaml
---
name: dream
description: >-
  Nightly memory consolidation. Collects sessions from the last 24h,
  extracts durable facts, dedupes, and routes into core markdown / RAG.

# Schedule
schedule: "0 3 * * *"
enabled: true
cwd: ~/.linggen                    # working directory for entry + agent
model: <optional override>
entry: scripts/collect.sh          # optional pre-agent script (relative to mission dir)

# Autonomy
allow-skills: []                   # whitelist for Skill tool — empty means mission calls no skill directly
requires: [memory]                 # optional — capabilities that must be registered at load

# Tools (SKILL.md shape)
allowed-tools:
  - Read
  - Write
  - Edit
  - Bash
  - Glob
  - Grep
  - Task
  - Memory_query
  - Memory_write
permission:
  paths:                           # per-path grants (same shape as SKILL.md)
    - { path: ~/.linggen/memory,     mode: write }
    - { path: ~/.linggen/sessions,   mode: write }
    - { path: ~/.claude/projects,    mode: read }
  warning: "..."                   # surfaced in UI
---

(step-by-step prompt body — same style as SKILL.md)
```

### Field reference

| Field | Required | Meaning |
|:------|:---------|:--------|
| `name` | yes | Mission id (matches directory name) |
| `description` | yes | Short human-readable summary — shown in UI |
| `schedule` | yes | Cron expression (5-field standard) |
| `enabled` | yes | On/off |
| `cwd` | yes | Working directory for entry script + agent |
| `model` | no | Model override |
| `entry` | no | Pre-agent script — path relative to mission dir, or inline `bash -c "..."` |
| `allow-skills` | no | Whitelist of skill names callable via `Skill`. Empty/omitted → `Skill` tool absent. `"*"` → any installed skill |
| `requires` | no | Capability names that must be resolvable at load time — else mission disabled with reason |
| `allowed-tools` | yes | Explicit tool list. `AskUser` / `EnterPlanMode` always stripped |
| `permission.mode` | yes | Capability ceiling on `cwd` and `paths` |
| `permission.paths` | no | Extra narrow path grants (like skill's `permission.paths`) |
| `permission.warning` | no | Displayed in the UI before enabling |

### Body IS the system prompt

A mission body is **the entire system prompt** for the run. No agent spec from `agents/<name>.md` is loaded on top — the mission is a self-contained instruction document. Tools come from `allowed-tools`, permission from `permission`, and the prompt body tells the LLM exactly what to do.

This is deliberate: missions are headless, scheduled, and rarely shared. The cost of one extra "agent persona" file per mission isn't earned. Authors write the runbook directly in `mission.md`; readers find all behaviour in one place.

The body should look like a SKILL.md body in structure — step-by-step instructions, explicit tool calls, output contract — but it stands on its own, without inheriting a persona block.

## Execution flow

```
 scheduler tick (every ~10s)
   │
   ▼
 cron match? ─── no ──► skip
   │ yes
   ▼
 busy-skip check (previous run still running?) ─── yes ──► record skipped
   │ no
   ▼
 run entry script (if present)
   │
   ├─ exit != 0 ──► mission failed; agent not invoked
   │ exit == 0
   ▼
 body present? ─── no ──► record completed (script-only mission)
   │ yes
   ▼
 create session + run agent loop with body as prompt
   │
   ▼
 finalize: record run + emit events + write runs.jsonl
```

### Entry script contract

When `entry:` is set, the scheduler runs it **before** invoking the agent. This replaces the old `script` mode and lets missions pre-compute expensive work (collecting session files, extracting raw material) cheaply — without burning LLM tokens.

Environment passed to entry:

| Var | Meaning |
|:----|:--------|
| `MISSION_ID` | Mission directory name |
| `MISSION_DIR` | Absolute path to the mission directory |
| `MISSION_CWD` | Resolved working directory (from `cwd:`) |
| `MISSION_OUTPUT_DIR` | Per-run scratch dir — scheduler creates it, entry writes to it, agent reads from it |
| `MISSION_LAST_RUN_AT` | Unix timestamp of the last successful run (or empty on first run) |
| `MISSION_RUN_ID` | Unique id for this run |

The script runs under the mission's permission bubble (same `allowed-tools`/`permission` constraints do **not** apply to entry — entry is shell, not an agent). Guardrails on entry are the mission author's responsibility.

Entry output conventions:
- **Structured data** → write to files under `$MISSION_OUTPUT_DIR/`. Agent `Read`s them in the body.
- **Stdout** → captured to `$MISSION_OUTPUT_DIR/stdout.log` as a fallback.
- **Stderr** → captured to `$MISSION_OUTPUT_DIR/stderr.log` for debugging.

If entry exits non-zero, the mission is marked failed and the agent loop is skipped. The captured logs are surfaced in the run record for diagnosis.

### Agent-only and script-only missions

- **Agent only** (no `entry:`) — classic prompt-driven mission. Same as today's agent mode.
- **Script only** (no body, `entry:` set) — pure background script. No LLM loop, no session, no cost. Replaces today's `mode: script`.
- **Hybrid** (entry + body) — entry pre-processes; agent consumes. Default for data-processing missions like `dream`.

The old `mode: app` (open a URL in browser on a schedule) is removed entirely. That use case is better served by a separate reminder feature, not missions.

## Cron syntax

Standard 5-field cron: `minute hour day-of-month month day-of-week`.

```
*/30 * * * *        → every 30 minutes
0 9 * * 1-5         → weekdays at 9am
0 0 * * 0           → every Sunday at midnight
0 */2 * * *         → every 2 hours
```

No seconds field. No `@reboot` or non-standard extensions.

## Permission model

Missions run without a human in the loop. Their permission model is the same path-mode model used by user sessions:

- **`permission.mode`** sets the capability ceiling on the mission `cwd` and every path in `permission.paths`.
- If the mission changes cwd, the effective mode is recomputed from those grants.
- If a mission needs more permission than its grants allow, it records a permission-needed failure/pause. It does not prompt the user during scheduled execution and does not silently widen access.

See `permission-spec.md` for the full model.

### Path-mode ceiling (`permission.mode`)

| Mode | Typical use |
|:-----|:------------|
| **read** | Monitoring, analysis missions |
| **edit** | Build / test / maintenance missions |
| **admin** | Trusted automation (memory, backups, system tasks) |

The mode applies to `cwd` plus every path under `permission.paths`. Skill grants loaded via `Skill` invocation compose via longest-path-match — a mission with `admin` on `~/.linggen` can safely invoke a skill that declares narrower `admin` on `~/.linggen/memory` without widening anything.

### Hardcoded deny floor

The engine's hardcoded deny floor (`sudo`, `rm -rf /`, forkbomb, etc.) applies to missions exactly as it applies to interactive sessions. There is no `linggen.toml` permission rule layer to inherit from. See `permission-spec.md`.

## Capability resolution

Missions consume tools and capabilities; skills register them.

- Skills declare `provides: [memory]` and `implements: { memory: { base_url: ..., tools: ... } }`. When a skill is installed, the engine registers its capability tools globally — any session (user, skill, mission) can call them.
- Missions list the capability tools they need (e.g. `Memory_write` (verb=add), `Memory_query` (verb=search)) directly in `allowed-tools`. They do **not** invoke the skill — they use the tools the skill registered.
- The `dream` mission uses `Memory_*` tools because the `ling-mem` skill registered them. `ling-mem` is the skill (slash command + capability provider); `linggen-memory` is the GitHub repo / Cargo crate that builds the `ling-mem` binary.

Missions never declare `implements:` themselves — the binding lives with the skill that registered the capability. If a capability isn't registered by any installed skill, `requires:` catches it at load; otherwise the tool call fails at runtime.

### Skill invocation via `Skill` tool

Separate from capability tools, a mission can delegate a whole sub-task to another skill via the `Skill` tool. This is the exception, not the rule. `allow-skills` gates it:

| Value | Effect |
|:------|:-------|
| omitted or `[]` | `Skill` tool absent from the effective set — mission calls no skill directly |
| `[skiller, ...]` | `Skill` tool added; only these skills invokable |
| `"*"` | `Skill` tool added; any installed skill invokable |

For the `dream` mission: `allow-skills: []`. It uses `Memory_*` tools directly, no skill invocation.

Invoked skills (when `allow-skills` is non-empty) inherit the **mission's** permission grants (mode + paths), not the skill's own defaults. A skill can't widen the mission's access by being called.

## Skill-bundled missions

Skills can ship missions as assets. The install script places them under `~/.linggen/missions/<name>/`:

```
skills/ling-mem/
├── SKILL.md
├── install.sh                  # copies assets/mission.md → ~/.linggen/missions/dream/mission.md
└── assets/
    └── mission.md              # the dream mission
```

Co-installation guarantees the dependency — the skill and its mission version together. This is the recommended pattern for domain-specific missions (memory → dream, backup → nightly-snapshot, etc.). For standalone missions authored by hand, `requires:` declares the dependency explicitly.

## Session per run

Every agent-mode run creates a new session. The session is the run log.

- **Session title**: `"Mission: <name> — <timestamp>"`.
- **All tool calls, messages, observations** recorded same as a user chat.
- **Viewable in UI**: runs appear in the session list (read-only).
- **Run entry links to session**: `MissionRunEntry.session_id` lets the UI navigate from run history to the full transcript.

Script-only missions (no body) do not create sessions. Their run record carries entry logs only.

## Scheduler behavior

Background task evaluates all enabled missions against their cron schedules every ~10 seconds:

1. **Tick** — wake, list enabled missions.
2. **Match** — for each, check if its cron expression matches the current minute window.
3. **Busy-skip** — if the previous run is still executing, record `skipped` and move on.
4. **Entry** — run the entry script if declared. Non-zero exit → fail fast, skip agent.
5. **Agent** — create session, construct prompt from body, run the agent loop.
6. **Record** — write `runs.jsonl` entry; emit events; finalize run record.

### Deduplication

The scheduler tracks the last fire minute per mission. A cron match only fires once per minute window — prevents double-firing on the same tick.

## Run history

Each trigger creates:

- A **session** (agent-mode runs only) containing the full conversation.
- An `AgentRunRecord` in `runs/` (standard format).
- A `mission_run` entry in `missions/<name>/runs.jsonl` linking run → mission → session.

```json
{
  "run_id": "mission-run-1700000000-a1b2c3d4",
  "session_id": "sess-1700000000-def",
  "triggered_at": 1700000000,
  "status": "completed",
  "skipped": false,
  "entry_exit_code": 0,
  "output_dir": "/Users/u/.linggen/missions/dream/runs/mission-run-1700000000-a1b2c3d4"
}
```

The mission-level `run_id` (format `mission-run-<ts>-<uuid8>`) keys the output dir, the `MISSION_RUN_ID` env var, and the `runs.jsonl` entry. It's distinct from the agent's internal `AgentRunRecord.run_id`, which stays an engine-internal concern.

Skipped triggers (busy / daily cap) are logged with `skipped: true` and no `session_id`; they still get a real `run_id` so downstream tooling can reference them. Script-only runs omit `session_id` and include `entry_exit_code`.

## Safety

| Guard | Value | Rationale |
|:------|:------|:----------|
| Minimum interval | 1 minute | Cron can't express sub-minute |
| Max triggers per mission | 100 per day | Caps runaway cost |
| Max concurrent missions | No hard limit | Busy-skip throttles naturally |
| `max_iters` | Per agent config | Bounds each triggered run |
| Path-mode grants | Required | Missions only run within their configured `cwd` and `permission.paths` |
| No interactive tools | — | `AskUser`/`EnterPlanMode` stripped regardless of `allowed-tools` |
| Hardcoded deny floor | Enforced | Engine-baked deny patterns block dangerous commands in every mode |
| Entry script failure | Skips agent | Prevents garbage-in agent work |

## Lifecycle

```
create → enabled → (triggers run on schedule, each run creates a session) → disabled → delete
```

- **Create** — user defines via Web UI, CLI, or hand-authored file. Skill-bundled missions created by `install.sh`.
- **Enable / disable** — toggle without deleting. Disabled missions keep config and history.
- **Delete** — removes the directory. Sessions created by past runs are preserved (they live in the global session store).
- **Edit** — update frontmatter or body. Takes effect on next tick. Entry script changes take effect on next run.

## UI

### Mission management page (Linggen Web UI)

- **List** — all missions with status, schedule, last run, next run.
- **Editor** — edit frontmatter fields + body. Body shown as markdown with step headings.
- **Permissions panel** — mode + paths + allow-skills + requires. Warnings from `permission.warning` surfaced before enable.
- **Agent tab** — read-only view of the mission body (prompt).
- **Run history** — list of `MissionRunEntry`; clicking a row opens the session read-only.
- **Manual trigger** — "Run now" button. Same permission bubble as scheduled runs.

### No in-mission UI

Missions do not render UI during execution. They have no chat partner to render for. Skills invoked from missions also do not render (the skill's app launcher is ignored when called from a mission context).

## API operations

| Operation | Description |
|:----------|:------------|
| List missions | All missions (with status, last run, next run) |
| Get mission | Full mission definition |
| Create mission | New mission (generates directory + `mission.md`) |
| Update mission | Edit frontmatter or body |
| Delete mission | Remove mission directory |
| Enable / disable | Toggle `enabled` flag |
| List runs | Run history for a mission (paginated) |
| Get run session | Read-only session view for a specific run |
| Get run output | Captured entry-script `stdout` / `stderr` for a specific run |
| Trigger mission | Fire now, ignoring schedule |

## Subsystem structure

Missions and skills are sibling subsystems inside linggen. They share shape (markdown + frontmatter), discovery (filesystem scan at startup + filewatcher), and runtime (agent engine + permission model). They differ only in trigger: skills are invoked on demand, missions are fired by cron.

| Concern | Skill subsystem | Mission subsystem |
|:--------|:----------------|:------------------|
| Root dir | `~/.linggen/skills/` | `~/.linggen/missions/` |
| Entry file | `SKILL.md` | `mission.md` |
| Trigger | User invocation or `Skill` tool call | Cron / manual trigger |
| Registers capabilities | Yes (`provides` + `implements`) | No (consumer only) |
| Interactive (`AskUser`, UI) | Yes | No |
| Stored under | `skills/<name>/` | `missions/<name>/` |
| Manager module | `skills/` | `project_store/missions.rs` |

Both subsystems are first-class — engine boot treats them symmetrically.

## Implementation

| Module | Responsibility |
|:-------|:---------------|
| `project_store/missions.rs` | Mission CRUD on disk, frontmatter parse/serialize, run history |
| `server/mission_scheduler.rs` | Cron evaluation, tick loop, entry execution, session creation, agent dispatch |
| `server/missions_api.rs` | HTTP endpoints for management and manual trigger |
| `engine/permission.rs` | Path-mode enforcement shared with interactive sessions |
| `skills/` | Capability registration — mission resolves tools through the same registry |

## Migration from old format

Mission permission no longer has a single top-level `mode` — every path declares its own. Missions also no longer get an implicit cwd grant; authors list every path they want granted, including cwd. Same shape as `SkillPermission`.

| Old field | New field |
|:----------|:----------|
| `permission.mode: admin` + `permission.paths: ["~/foo"]` | `permission.paths: [{path: ~/foo, mode: admin}]` |
| `permission_tier: readonly` (legacy flat field) | drop the field; rewrite `permission.paths` per-entry with `mode: read` |
| `permission_tier: standard` | per-entry `mode: edit` (alias `write`) |
| `permission_tier: full` | per-entry `mode: admin` |
| Implicit cwd grant (was applied at `permission.mode`) | list cwd explicitly in `permission.paths` if you need it |
| `mode: agent` | *(removed — default)* |
| `mode: script` | remove `mode`, move command to `entry:`, clear body |
| `mode: app` | **dropped — no migration path**; authors convert to an external reminder |
| top-level `prompt` | markdown body below frontmatter |
| `agent_id` | *(removed — mission body IS the agent's system prompt; no `agents/<name>.md` is loaded for missions)* |

Note: the parser no longer auto-converts the old shape. Mission files still using the legacy `permission_tier` or single `permission.mode` will load with **no permission grants** and likely fail the first time they try to touch the filesystem. Rewrite them to the per-path shape.
