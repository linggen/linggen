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

**Mission = scheduled task.** Cron-triggered. Consumes tools and capabilities that skills register. Renders no UI of its own — the agent communicates through the session transcript (visible after the run via the mission's run history). Whether a mission can ask the user is up to its `allowed-tools`: a scheduled overnight mission like `dream` omits `AskUser` because there's nobody to answer; a manually-triggered or catch-up mission may list `AskUser` if its author wants it to.

A mission looks like a `SKILL.md` with a `schedule:` field. It uses built-in engine tools plus capability tools (like `Memory_*`) that installed skills have registered, calling them directly. Missions do **not** delegate to skills — the `Skill` tool is never part of a mission's tool surface (see "Tools missions can use").

## File layout

Missions live under `~/.linggen/missions/` and mirror the skill directory shape:

```
~/.linggen/missions/dream/
├── mission.md         # frontmatter + agent prompt (body)
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
catchup_hours: 24                  # optional — fire from the post-turn seam if last run is older than this
enabled: true
agent: ling-mem                    # optional — engine agent to run this mission (default: ling)
cwd: ~/.linggen                    # working directory for the agent
model: <optional override>

# Kickoff: one or more user-turn messages persisted into the session
# at the start of the run. Item 0 fires immediately; item 1+ drain
# one-per-assistant-final-reply via the engine's kickoff_queue.
kickoff:
  - "Briefly greet and say what you're about to do."
  - "Now do the actual work, per your system prompt."

# Tools (SKILL.md shape)
allowed-tools:
  - Read
  - Write
  - Edit
  - Bash
  - Glob
  - Grep
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
| `catchup_hours` | no | If set, the post-turn seam fires the mission when its last non-skipped run is older than this many hours. Used to recover from cron fires missed while the machine was off/asleep. Omit to leave the mission cron-only. `0` is treated as opt-out |
| `enabled` | yes | On/off |
| `agent` | no | Engine agent that runs the mission (key into `agents/`). Defaults to `ling`. The mission body is still the system prompt — `agent:` just picks the routing identity, the model default, and the persona-level config |
| `cwd` | yes | Working directory for the agent |
| `model` | no | Model override |
| `kickoff` | no | Ordered list of user-turn messages persisted at run start. Item 0 fires immediately; later items drain one-per-assistant-final-reply via the engine's `kickoff_queue`. Empty list falls back to a single generic "Run the X mission" line. Use for staged onboarding (greet → start work) without batching everything into one model reply |
| `allowed-tools` | yes | Explicit tool list. Authors omit `AskUser` / `EnterPlanMode` for unattended missions where there's no one to respond. The `Skill` tool is never available to missions — missions do not delegate to skills |
| `permission.paths` | no | Per-path grants, each with its own `mode`. Same shape as `SkillPermission` — see `permission-spec.md`. Omitting `permission` or leaving `paths` empty means the mission has no filesystem grants and will fail the first write/edit it attempts |
| `permission.warning` | no | Displayed in the UI before enabling |

### Body + agent identity together form the system prompt

A mission body is the **runbook**: step-by-step instructions, explicit tool calls, output contract. It mirrors a SKILL.md body in shape.

The agent referenced in frontmatter (`agent: ling`, default `ling`) contributes its `## Identity` block — the first paragraph of its spec body (typically `"You are X — <short self-description>"`) plus the YAML `personality` field. This block is prepended to the mission body, giving the run a consistent voice without forcing every mission author to duplicate "You are Ling" prose.

The agent's **spec body** (workflow, delegation, planning, memory protocol, etc.) is NOT concatenated — that guidance is tuned for interactive coding sessions, not for headless cron. Missions get persona + body only.

So the assembled system prompt for a mission is:

```
## Identity
<agent.personality + agent spec's first-paragraph identity preface>

<mission body — verbatim, with $MISSION_DIR substituted>
```

Tools come from `allowed-tools`, permission from `permission`. Everything else the LLM needs to know is in the mission body.

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
 create session + persist kickoff[0] as user turn
   │
   ▼
 run agent loop with body as system prompt; on each assistant
 final-response, drain next kickoff item from queue as the next
 user turn (until queue empty), then end
   │
   ▼
 finalize: record run + emit events + write runs.jsonl
```

### Kickoff queue

`kickoff:` is a list of user-turn messages that seed the session before any agent reply. Item 0 is persisted as the first chat message and becomes the agent's initial task. Items 1+ go into the engine's `kickoff_queue`; the agent loop drains one each time it emits a final assistant reply (a turn with no further tool calls). This staged delivery lets a mission produce a greeting *as its own* assistant message before the work-step kickoff arrives, instead of folding everything into one batched reply.

If `kickoff:` is omitted or empty, the scheduler falls back to a single generic line (`Run the "<name>" mission per your system prompt.`).

There is no pre-agent shell stage. Deterministic data fetches happen inside the agent loop via the mission's `allowed-tools` (typically a built-in capability tool like `Memory_query`, or `Bash` when the mission declares it). For missions that previously relied on a shell pre-fetch to dodge LLM-judgment risk on empty results, the protection now lives at the dispatch boundary in `engine/capabilities.rs` — see the ling-mem `past_ttl=true` strip rule.

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

Missions run without a human in the loop. Their permission model is the same per-path mode model used by skills and user sessions:

- **`permission.paths`** is a list of `{path, mode}` grants — same shape as `SkillPermission`. Each path declares its own mode.
- No implicit grant on `cwd`. If the mission needs write access to its working directory, list `cwd` explicitly in `permission.paths`.
- If a mission needs more permission than its grants allow, it records a permission-needed failure/pause. It does not prompt the user during scheduled execution and does not silently widen access.

See `permission-spec.md` for the full model.

### Per-path mode

| Mode | Typical use |
|:-----|:------------|
| **read** | Monitoring, analysis missions |
| **edit** (alias `write`) | Build / test / maintenance missions |
| **admin** | Trusted automation (memory, backups, system tasks) |

Grants compose via longest-path-match — the most-specific `{path, mode}` entry covering a target wins.

### Hardcoded deny floor

The engine's hardcoded deny floor (`sudo`, `rm -rf /`, forkbomb, etc.) applies to missions exactly as it applies to interactive sessions. There is no `linggen.toml` permission rule layer to inherit from. See `permission-spec.md`.

## Tools missions can use

Missions and skills are independent subsystems — a mission **cannot** delegate to a skill, and the `Skill` tool is not part of any mission's tool surface. A mission lists what it needs in `allowed-tools` and the engine resolves each name against:

1. **Built-in engine tools** — `Read`, `Write`, `Edit`, `Bash`, `Glob`, `Grep`, `WebFetch`, `WebSearch`, `Task`, etc.
2. **Built-in capability tools** — e.g. `Memory_query` / `Memory_write`. These ship with the engine and dispatch directly to the daemon URL in `agent.ling_mem_url` (see `engine/capability_tools.rs` and `engine/capabilities.rs`). The `ling-mem` daemon is installed alongside `ling` by the Linggen installer; no skill is consulted.

If a listed tool name doesn't resolve to either bucket, the call fails at runtime with `unknown tool: <name>`. There is no separate `requires:` field — `allowed-tools` is the complete contract.

The `dream` mission lists `Bash`, `Memory_query`, `Memory_write` — all built-ins. It runs without any installed skill being present.

## Session per run

Every run creates a new session. The session is the run log.

- **Session title**: `"Mission: <name> — <timestamp>"`.
- **All tool calls, messages, observations** recorded same as a user chat.
- **Viewable in UI**: runs appear in the session list (read-only).
- **Run entry links to session**: `MissionRunEntry.session_id` lets the UI navigate from run history to the full transcript.

## Scheduler behavior

Background task evaluates all enabled missions against their cron schedules every ~10 seconds:

1. **Tick** — wake, list enabled missions.
2. **Match** — for each, check if its cron expression matches the current minute window.
3. **Busy-skip** — if the previous run is still executing, record `skipped` and move on.
4. **Agent** — create session, persist `kickoff[0]` as the first user turn, seed `kickoff[1..]` into the engine queue, run the agent loop with the body as system prompt.
5. **Record** — write `runs.jsonl` entry; emit events; finalize run record.

### Deduplication

The scheduler tracks the last fire minute per mission. A cron match only fires once per minute window — prevents double-firing on the same tick.

### Catch-up fires

Cron is missed when the machine is off or asleep. To recover, a mission can declare `catchup_hours: <n>` in its frontmatter. After every owner-session user turn, a non-blocking sweep walks all enabled missions whose last non-skipped run is older than their `catchup_hours` and triggers them — same dispatch path as a normal cron fire. Catch-up overlaps are prevented by the same busy-skip used for cron. Missions that omit `catchup_hours` (or set it to `0`) are cron-only.

The built-in `dream` consolidation mission uses `catchup_hours: 24`: missed 3am fires re-run opportunistically the next time the user sends a turn.

## Run history

Each trigger creates:

- A **session** containing the full conversation.
- An `AgentRunRecord` in `runs/` (standard format).
- A `mission_run` entry in `missions/<name>/runs.jsonl` linking run → mission → session.

```json
{
  "run_id": "mission-run-1700000000-a1b2c3d4",
  "session_id": "sess-1700000000-def",
  "triggered_at": 1700000000,
  "status": "completed",
  "skipped": false
}
```

The mission-level `run_id` (format `mission-run-<ts>-<uuid8>`) keys the `runs.jsonl` entry. It's distinct from the agent's internal `AgentRunRecord.run_id`, which stays an engine-internal concern.

Skipped triggers (busy / daily cap) are logged with `skipped: true` and no `session_id`; they still get a real `run_id` so downstream tooling can reference them.

## Safety

| Guard | Value | Rationale |
|:------|:------|:----------|
| Minimum interval | 1 minute | Cron can't express sub-minute |
| Max triggers per mission | 100 per day | Caps runaway cost |
| Max concurrent missions | No hard limit | Busy-skip throttles naturally |
| `max_iters` | Per agent config | Bounds each triggered run |
| Path-mode grants | Required | Filesystem access is limited to the `{path, mode}` entries in `permission.paths`. No implicit grant on `cwd` |
| Interactive tools | Author opt-in | `AskUser` / `EnterPlanMode` only available when listed in `allowed-tools`. Scheduled cron missions should omit them; manually-triggered missions may include them when a user is reachable |
| Hardcoded deny floor | Enforced | Engine-baked deny patterns block dangerous commands in every mode |

## Lifecycle

```
create → enabled → (triggers run on schedule, each run creates a session) → disabled → delete
```

- **Create** — user defines via Web UI, CLI, or hand-authored file. Built-in missions (e.g. `dream`) are installed by the Linggen installer alongside the engine.
- **Enable / disable** — toggle without deleting. Disabled missions keep config and history.
- **Delete** — removes the directory. Sessions created by past runs are preserved (they live in the global session store).
- **Edit** — update frontmatter or body. Takes effect on next tick. Entry script changes take effect on next run.

## UI

### Mission management page (Linggen Web UI)

- **List** — all missions with status, schedule, last run, next run.
- **Editor** — edit frontmatter fields + body. Body shown as markdown with step headings.
- **Permissions panel** — `permission.paths` (per-path mode). Warnings from `permission.warning` surfaced before enable.
- **Agent tab** — read-only view of the mission body (prompt).
- **Run history** — list of `MissionRunEntry`; clicking a row opens the session read-only.
- **Manual trigger** — "Run now" button. Same permission bubble as scheduled runs.

### In-run UI

Missions render no UI of their own; the run shows up in the mission's run history and opens as a read-only session transcript. If the mission lists `AskUser` in `allowed-tools` and a user is reachable (manual trigger, catch-up run, or any path where a chat surface is bound), the question routes to the session's chat panel like any other agent question. Skill app launchers invoked from a mission are ignored — missions don't open windows.

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
| Trigger mission | Fire now, ignoring schedule |

## Subsystem structure

Missions and skills are sibling subsystems inside linggen. They share shape (markdown + frontmatter), discovery (filesystem scan at startup + filewatcher), and runtime (agent engine + permission model). They differ only in trigger: skills are invoked on demand, missions are fired by cron.

| Concern | Skill subsystem | Mission subsystem |
|:--------|:----------------|:------------------|
| Root dir | `~/.linggen/skills/` | `~/.linggen/missions/` |
| Entry file | `SKILL.md` | `mission.md` |
| Trigger | User invocation or `Skill` tool call | Cron / manual trigger |
| Registers capabilities | Yes (`provides` + `implements`) | No (consumer only) |
| Interactive (`AskUser`, UI) | Yes | Opt-in via `allowed-tools`; no skill-style app launcher |
| Stored under | `skills/<name>/` | `missions/<name>/` |
| Manager module | `skills/` | `project_store/missions.rs` |

Both subsystems are first-class — engine boot treats them symmetrically.

## Implementation

| Module | Responsibility |
|:-------|:---------------|
| `engine/mission/record.rs` | `Mission`, `MissionRunEntry`, `MissionPermission` runtime records |
| `engine/mission/registry.rs` | `MissionRegistry` trait (spec lookup contract) |
| `engine/mission/runs.rs` | `MissionRunStore` trait (run-history persistence contract) |
| `extensions/missions/mod.rs` | `MissionLoader` — disk CRUD, frontmatter parse/serialize, run history; impls both engine traits |
| `extensions/missions/scheduler.rs` | Cron evaluation, tick loop, session creation, kickoff seeding, agent dispatch |
| `server/api/missions.rs` | HTTP endpoints for management and manual trigger |
| `engine/permission/` | Path-mode enforcement; `manifest.rs` owns the YAML grant grammar shared with skills |
| `extensions/skills/` | Capability registration — mission resolves tools through the same registry |
| `extensions/{frontmatter,script,scope}.rs` | Helpers both skills and missions use (YAML splitter, bash launcher, tool-scope) |

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
| `mode: script` | **dropped** — every mission now needs a body; convert the script's work into an agent-driven `Bash`/capability-tool call, or remove the mission |
| `mode: app` | **dropped — no migration path**; authors convert to an external reminder |
| `entry:` (top-level) | **dropped** — author the work as agent steps in the body; if you need to seed input, put it in `kickoff:` |
| top-level `prompt` | markdown body below frontmatter |
| `agent_id` | *(removed — mission body IS the agent's system prompt; no `agents/<name>.md` is loaded for missions)* |

Note: the parser no longer auto-converts the old shape. Mission files still using the legacy `permission_tier` or single `permission.mode` will load with **no permission grants** and likely fail the first time they try to touch the filesystem. Rewrite them to the per-path shape.
