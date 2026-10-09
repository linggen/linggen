---
type: spec
reader: Coding agent and users
guide: |
  Product specification — describe what the system should do and why.
  Keep it brief. Aim to guide design and implementation, not document code.
  Avoid implementation details like function signatures, variable types, or code snippets.
---

# Agents

Process management: agent types, lifecycle, delegation, concurrency, and scheduling.

## Related docs

- `session-spec.md`: session/context model, effective tools, system prompt assembly.
- `agentic-loop.md`: the loop each agent runs.
- `chat-spec.md`: events for agent status.
- `product-spec.md`: mission system overview.

## Agent types

Agents are discovered dynamically from `agents/*.md` markdown files. No hardcoded roster.

- **Main agents**: long-lived, receive user tasks, can delegate.
- **Subagents**: ephemeral child workers, spawned by delegation.

### Frontmatter fields

| Field | Required | Purpose |
|:------|:---------|:--------|
| `name` | yes | Agent identity |
| `description` | yes | What the agent does (used for discovery and delegation) |
| `tools` | yes | Tool declarations (used for prompt assembly). The session's effective path mode controls actual access — see `permission-spec.md`. |
| `personality` | no | Response style guide — concise directive for HOW the agent communicates |
| `aliases` | no | Other names a message may address it by at its start (`@银月 …`), beside its id |
| `internal` | no | `true` = not addressable by a person: left out of the chat's `@` / `@@` lists and never the target of a leading `@name`. The engine still runs it (missions, delegation). |
| `met_when` | no | `{skill, file, path}` — the agent appears only once met: absent from every surface the engine controls until the value at `path` in the skill's JSON `file` is set (same reading as a skill's `absent_until`, `skill-spec.md` § Place). Absent: always present. See *Appearing only once met* below. |

Runtime configuration (model, effective tools, bound skill) is set at the session level. See `session-spec.md`.

### Appearing only once met

An agent that declares `met_when` (Yinyue: Lingjing's `data/state.json`, `companion.joined`) is **absent until met, and met for good**. The engine names no agent and no skill — it reads the declaration and resolves the skill's folder through the skill registry.

- **The latch** is a one-way file, `~/.linggen/met/<agent>.json` (`{"at": RFC 3339, "reason": "trigger" | "history"}`; `storage-spec.md`). Written once, never cleared — a save forgotten, a game restarted or the skill removed leaves her met. It is not in the skill's folder, not in the save, not under `agents/` (rewritten on every start).
- **Trigger.** The declared state read set latches it (`reason: trigger`): checked at startup, on a 3 s tick while the agent waits, and at once whenever a surface asks.
- **Grandfather.** At startup, before the server binds, an agent that already has chat history with this person — a line it spoke itself in its own `sess-<agent>-…` thread or in any session that seats it — is latched (`reason: history`). An upgrade never makes her vanish; a fresh install follows the rule strictly.
- **The gate** is `AgentManager::agent_present` (sync) / `agent_present_now` (reads the trigger first). Unmet, the agent is left out of the agent list, `/api/agent-files`, the `@` picker and leading-`@` resolution; a message to it by id is refused (`status: "absent"`); its own turns, heralds, ambient glances, app moments, speech cues, presenter registration and `/mute` do not run; Ling's prompt and `agent_chat` do not speak of her. Inside a skill that gates the agent itself (`absent_until`), the skill's gate stays authoritative; at any other skill's table the agent is there only once met.
- **Exposed** as page_state `agents_met` (`[{id, present, met: {at, reason} | null}]`) and the retained `mac/agents` topic (`{agents: [...same], host, published_at}`) for the phone and the Mac shell. Only agents that declare `met_when` are listed.
- Permission prompts, questions and run failures never routed through her: they are widgets and chat lines on the plain UI path, so a fresh install needs nothing of her. Her heralds (an away callback) are voice only and simply do not happen before she is met.

### Voice: opt-in, not universal

A plain user session answers in the model's own default style; the engine
injects no voice text (2026-10-04). `agents/shared/voice.md` — plain, human,
no robot tells — stays as a fragment an agent, skill or app opts into with
`{{#include shared/voice.md}}` when it wants that style. An agent with a
deliberate voice (Yinyue) writes it in its own spec.

`agents/shared/humanize.md` is the long-form companion: the full catalog of
AI-writing tells with before/after fixes. Not injected — skills that draft
outward-facing text read it at drafting time from
`~/.linggen/agents/shared/humanize.md`, where the installer guarantees it
on every daemon start.

### Shared body includes

A body line of exactly `{{#include path.md}}` (mdBook syntax) is replaced at
load with that file's content, resolved relative to the including file.
Includes nest, cycles are errors, and a missing file fails the spec loudly.
Shared fragments live in `agents/shared/` — subdirectories are not scanned
for agents, and the installer re-syncs them on every daemon start. This is a
general facility; the built-in agents do not include `voice.md`, but any
agent can include any shared fragment.

### Ling: the general-purpose agent

Ling is the primary agent — like Jarvis, one agent that adapts to any context. Skills shape ling's behavior:

- **Skill body** → injected into system prompt as domain instructions
- **Skill `allowed-tools`** → narrows ling's `tools: ["*"]`
- **Dynamic prompt** → engine only includes instructions for capabilities available

When a skill sets `allowed-tools: []`, tool-related prompt sections (schemas, usage guidelines, delegation, plan mode) are skipped. Ling becomes a pure conversational agent shaped by the skill's instructions.

### Default agents

| Agent | Role | Key tools |
|:------|:-----|:----------|
| `ling` | The universal user-facing agent — adapts to any context via skills | `["*"]` |

Memory consolidation is **not** done by a separate subagent anymore. `ling`
itself writes durable rows inline (driven by the system prompt's memory
protocol + the per-turn auto-recall hint), matching how Claude Code and
Codex operate. The user-triggered `dream` mission still handles bulk
reprocessing — see [`memory-spec.md`](memory-spec.md) §2.

Ling is the universal agent. Specialized behavior comes from skills:
- **Game-table skill** — bound to game sessions, zero tools, pure conversation
- **Linggen-guide skill** — documentation lookup and Q&A
- **Apple Shifu skill** — system health analyst with web dashboard
- **Shared-memory skill** — the memory store's CLI and Data Browser (the tools themselves come from ling-mem's MCP server, not the skill)

Missions are a sibling subsystem to skills (cron-scheduled), not a skill type. See `mission-spec.md`.

### Dynamic system prompt

The system prompt is assembled from layers:

1. **Personality** (from agent frontmatter) — always present, sets response style
2. **Agent body** (from agent .md) — always present, sets identity and adaptation rules
3. **Skill frame** (from active SKILL.md) — when skill is active
4. **Environment** — platform, workspace
5. **Project instructions** (CLAUDE.md) — when present
6. **Tool schemas + guidelines** — only when effective tools is non-empty
7. **Delegation targets** — only when Task tool is available
8. **Memory** — only when Write tool is available

This means a game session (no tools) gets a minimal, focused prompt. A coding session gets the full tool-aware prompt. Same agent, different context.

## Lifecycle

```
created → running → completed | failed | cancelled
```

Each execution is an `AgentRunRecord`:
- `run_id`, `repo_path`, `session_id`, `agent_id`
- `agent_kind` (main | subagent)
- `parent_run_id` (for delegated runs)
- `status`, `detail`, `started_at`, `ended_at`

**Implementation**: `engine/agent/mod.rs`

## Delegation (fork)

`Task` spawns a child agent loop — like `fork()`:

- Parent collects consecutive delegations, spawns concurrently via `JoinSet`.
- Each child gets its own isolated engine and tokio runtime.
- Parent waits for all children, feeds results back into its context.
- Depth tracked and limited by `agent.max_delegation_depth` (default 2).
- Cancellation cascades from parent to children.

**Delegation rules**:
- `main → main` messaging: allowed.
- `main → subagent` delegation: allowed.
- `subagent → parent` return: allowed.
- `subagent → subagent`: denied.
- `subagent → spawn(*)`: denied.

## Capabilities

Capabilities are determined by which tools are available in the session — no separate policy system. See `session-spec.md` for the effective tools model.

Tool access is hard-gated by the session's effective tool set. Wildcard `tools: ["*"]` means unrestricted.

## Mission system (scheduler)

Cron-based scheduled agent work. A project can have **multiple active missions** — each with its own cron schedule, target agent, and prompt.

- **No missions** → agents purely reactive.
- **Active missions** → agents triggered on cron schedules.

See [`mission-spec.md`](mission-spec.md) for full details: cron syntax, scheduling behavior, safety guards, run history, and API.

## Concurrency model

- Each agent holds a lock during execution (`agent.try_lock()`).
- Multiple agents can run concurrently (different locks).
- Messages to a busy agent are queued (see `agentic-loop.md`).
- Subagents spawned via delegation run on separate tokio runtimes.
