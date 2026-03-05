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

- `agentic-loop.md`: the loop each agent runs.
- `chat-spec.md`: SSE events for agent status.
- `product-spec.md`: mission system overview.

## Agent types

Agents are discovered dynamically from `agents/*.md` markdown files. No hardcoded roster.

- **Main agents**: long-lived, receive user tasks, can delegate.
- **Subagents**: ephemeral child workers, spawned by delegation.

Determined by frontmatter `kind: main` or `kind: subagent`.

**Frontmatter fields**: `name`, `description`, `tools`, `model`, `kind`, `work_globs`, `policy`.

### Default agents

| Agent | Role | Key tools |
|:------|:-----|:----------|
| `ling` | General-purpose assistant, delegates specialist work | Read, Glob, Grep, Bash, Task, AskUser |
| `coder` | Implementation — writes and edits code | Read, Write, Edit, Bash, Glob, Grep, AskUser |
| `explorer` | Read-only codebase exploration | Read, Glob, Grep, Bash |
| `debugger` | Read-only debugging and log analysis | Read, Glob, Grep, Bash |
| `linggen-guide` | Linggen docs and usage guide | Read, Glob, Grep, Bash, WebSearch, WebFetch |

`ling` auto-delegates to `linggen-guide` when users ask about Linggen itself.

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

## Policy gates

Per-agent capabilities configured in frontmatter:

- `Patch` — can apply patches.
- `Finalize` — can finalize tasks.
- `Delegate` — can delegate to other agents.
- `policy.delegate_targets` — constrains which agents can be delegated to.

Tool access is hard-gated by `spec.tools`. Wildcard `tools: ["*"]` means unrestricted.

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
