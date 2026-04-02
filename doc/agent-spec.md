---
type: spec
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
- `chat-spec.md`: SSE events for agent status.
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
| `tools` | yes | Tool declarations (used for prompt assembly). Session permission mode controls actual access — see `permission-spec.md`. |
| `personality` | no | Response style guide — concise directive for HOW the agent communicates |

Runtime configuration (model, effective tools, bound skill) is set at the session level. See `session-spec.md`.

### Ling: the general-purpose agent

Ling is the primary agent — like Jarvis, one agent that adapts to any context. Skills shape ling's behavior:

- **Skill body** → injected into system prompt as domain instructions
- **Skill `allowed-tools`** → narrows ling's `tools: ["*"]`
- **Dynamic prompt** → engine only includes instructions for capabilities available

When a skill sets `allowed-tools: []`, tool-related prompt sections (schemas, usage guidelines, delegation, plan mode) are skipped. Ling becomes a pure conversational agent shaped by the skill's instructions.

### Default agents

| Agent | Role | Key tools |
|:------|:-----|:----------|
| `ling` | The only agent — adapts to any context via skills | `["*"]` |

Ling is the universal agent. Specialized behavior comes from skills:
- **Mission skill** — bound to cron sessions, sets autonomous execution mode with safety guardrails
- **Game-table skill** — bound to game sessions, zero tools, pure conversation
- **Linggen-guide skill** — documentation lookup and Q&A

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

**Implementation**: `agent_manager/mod.rs`

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
