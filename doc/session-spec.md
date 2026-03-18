---
type: spec
guide: |
  Product specification — describe what the system should do and why.
  Keep it brief. Aim to guide design and implementation, not document code.
  Avoid implementation details like function signatures, variable types, or code snippets.
---

# Session & Context

A session is the fundamental unit of interaction. Each session holds one context — an isolated conversation with its own tools, system prompt, and message history.

## Related docs

- `chat-spec.md`: SSE events, rendering, message model.
- `agent-spec.md`: agent definitions, delegation.
- `skill-spec.md`: skill format, activation.
- `agentic-loop.md`: how the engine runs inside a context.

## Core concept

Session = Context. One session, one context. They are the same thing.

A session is created by a **creator**, which determines the session's initial configuration — tools, system prompt, model. The creator copies its spec into the session. After creation, the session's configuration can change dynamically during the conversation.

## Creators

Every session has a creator that determines how it was born and what capabilities it starts with.

| Creator | How it's created | Typical tools | Example |
|:--------|:-----------------|:-------------|:--------|
| `user` | User clicks "New Chat" in UI or sends first message | All agent tools (`["*"]`) | Normal coding session |
| `skill` | Skill app creates a session via API with `skill` field | Defined by skill's `allowed-tools` | Game-table creates a zero-tool session |
| `agent` | Sub-agent spawned via `Task` tool | Inherits parent's tools | Ling delegates exploration to itself |
| `mission` | Cron scheduler triggers a mission run | Mission agent's tools | Nightly code review mission |

### Creator inheritance

When a session is created, it inherits configuration from its creator:

1. **Agent tools** — base tool set from the agent spec (e.g., `ling` has `["*"]`)
2. **Skill restriction** — if a skill is bound, `effective_tools = intersection(agent.tools, skill.allowed-tools)`
3. **System prompt** — assembled from agent personality + agent body + skill body (if bound)
4. **Model** — from the agent spec or mission config, falling back to the default routing chain

## Session state

Each session carries:

| Field | Set at | Mutable | Purpose |
|:------|:-------|:--------|:--------|
| `id` | creation | no | Unique identifier (`sess-{timestamp}-{uuid8}`) |
| `title` | creation | yes | Human-readable name, can be auto-generated |
| `created_at` | creation | no | Unix timestamp |
| `creator` | creation | promotion only | Who owns this session: `user`, `skill`, `agent`, `mission`. Changes from `mission` → `user` on promotion. |
| `skill` | creation | no | Bound skill name (if any) — active for the session's lifetime |
| `agent_id` | creation | yes | Which agent runs in this session |
| `model` | creation | yes | Model override for this session |
| `messages` | runtime | append | Chat history (`messages.jsonl`) |

### What's dynamic (can change during chat)

- **Model** — user can switch models mid-conversation via `/model`
- **Agent** — user can switch agents mid-conversation via `/agent` (tools change accordingly)
- **System prompt** — rebuilt each turn from current agent + skill + environment + project instructions
- **Title** — can be renamed

### What's fixed for the session lifetime

- **Bound skill** — set at creation, cannot be changed. Start a new session for a different skill.
- **Creator** — immutable except for promotion (mission → user when a human takes over).

## System prompt assembly

The system prompt is rebuilt each turn from layers. Which layers are included depends on the session's current state:

```
[1]  Agent personality       — always (sets response style)
[2]  Agent body              — always (sets identity and behavior)
[3]  Available skills        — skill names + descriptions for discovery
[4]  Active skill body       — when session has a bound skill
[5]  Environment             — always (platform, workspace, date)
[6]  Project instructions    — when CLAUDE.md exists
[7]  Memory                  — when MEMORY.md exists and Write tool available
[8]  Tool schemas            — when effective_tools is non-empty
[9]  Plan mode instructions  — when plan_mode is active
[10] Delegation targets      — when Task tool is in effective_tools
[11] Plan execution context  — when plan is approved/executing
```

A zero-tool session (e.g., game-table skill with `allowed-tools: []`) gets layers 1-7 only — a minimal, focused prompt for pure conversation.

## Effective tools

The tools available in a session are determined by the skill when bound:

```
effective_tools = skill.allowed_tools              (if skill is bound)
                  agent.tools                       (if no skill)
```

- `agent.tools: ["*"]` means all built-in tools.
- `skill.allowed-tools: []` means no tools at all.
- `skill.allowed-tools: ["Read", "Grep"]` means only those tools.
- No `allowed-tools` field means no restriction (falls through to agent tools).

Capabilities are determined by which tools are present:

| Capability | Requires tool |
|:-----------|:-------------|
| Read files | `Read` |
| Write/edit files | `Write`, `Edit` |
| Run commands | `Bash` |
| Search code | `Grep`, `Glob` |
| Delegate to sub-agent | `Task` |
| Web access | `WebSearch`, `WebFetch` |
| Invoke skills | `Skill` |
| Ask the user | `AskUser` |

No separate policy system — the tool list IS the permission model.

## Storage

Sessions live under their creator's namespace:

| Creator | Session path |
|:--------|:------------|
| `user` | `~/.linggen/projects/{encoded}/sessions/{session_id}/` |
| `skill` | `~/.linggen/skills/{skill_name}/sessions/{session_id}/` |
| `mission` | `~/.linggen/missions/{mission_id}/sessions/{session_id}/` |
| `agent` | Ephemeral — not persisted to disk |

Each persisted session directory contains:

```
{session_id}/
  session.yaml      # SessionMeta (id, title, created_at, creator, skill, ...)
  messages.jsonl     # one ChatMsg per line, append-only
```

See `storage-spec.md` for the full filesystem layout.

## Session promotion (mission → user)

When a user sends a message to a mission-created session, the session is **promoted** to a user session. The mission ran autonomously; now the user is taking over the conversation interactively.

### What triggers promotion

A `POST /api/chat` request where:
- `mission_id` is present (the UI is viewing a mission session), AND
- the message comes from a real user (not the mission scheduler)

The chat handler detects this by checking the session's current `creator` field. If it's `"mission"`, promotion happens before the agent loop runs.

### What changes on promotion

| Aspect | Before (mission) | After (user) |
|:-------|:-----------------|:-------------|
| `creator` field | `"mission"` | `"user"` |
| `tool_permission_mode` | `Auto` (forced by scheduler) | Config default (from `linggen.toml`) |
| `mission_allowed_tools` | Set by permission tier | Cleared |
| `bash_allow_prefixes` | Set by permission tier | Cleared |
| System prompt | Rebuilt from `ling.md` | Same — but cache invalidated to pick up any config changes |
| Chat history | Mission messages | Preserved — user sees full mission context |
| Session files | Stay in `~/.linggen/missions/{mission_id}/sessions/` | No move — files stay in place |

### What stays the same

- **Session ID** — unchanged.
- **Message history** — all mission messages are preserved. The user's conversation builds on top of the mission's context.
- **Session file location** — files remain in the mission sessions directory. No file moves.
- **Agent** — still `ling`. The mission agent IS ling.

### Why not create a new session?

The user wants to continue the conversation — give feedback on the mission's plan, ask follow-up questions, approve/reject work. Creating a new session would lose the mission context. Promotion keeps the context and switches the interaction mode.

### Promotion is one-way and permanent

Once promoted, the session stays as `"user"`. If the mission scheduler needs to run again, it creates a new session.

## Session lifecycle

```
created → active → idle
                 → promoted (mission → user, on first user message)
                 → archived (future: manual or auto-cleanup)
```

- **Created**: session.yaml written, empty messages file.
- **Active**: agent is running, messages being appended.
- **Idle**: no active agent run. Resumes on next user message.
- **Promoted**: mission session taken over by user. Creator updated, engine reconfigured.

## Sub-agent sessions

When an agent delegates via `Task`, the child runs in its own context (isolated message history, fresh token budget). This is effectively a child session:

- Inherits the parent's model and delegation depth counter.
- Has its own message history (not persisted to disk — ephemeral).
- Results are returned to the parent's context as a tool result.
- The parent's context stays clean — only the summary comes back.

This is how `ling` can delegate exploration to itself: the sub-task runs in an isolated context, does the work, and returns a concise summary.

## API

Session CRUD is part of the projects API:

- `POST /api/sessions` — create session (accepts `skill`, `title`)
- `GET /api/sessions` — list sessions for a project
- `DELETE /api/sessions/:id` — delete a session
- `POST /api/chat` — send message to a session (creates session if needed)
