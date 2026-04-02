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

## Working folder

Every session has a **working folder** (cwd) — the directory the agent operates in. The working folder replaces the old "project selection" concept.

### Home path

The **home path** is the default working folder for new sessions. Defaults to `~`. Users can change it in settings (e.g. `~/workspace`, `~/projects`). Stored in `linggen.toml` as `home_path`.

### How it works

1. **New chat** — starts at the home path. No project picker — just click `+` and go.
2. **Agent or user runs `cd`** — after any bash command, the backend checks the new cwd.
3. **Git detection** — if `.git/` exists in the cwd or any parent, the session enters **project mode** for that git root. The engine loads the project's `CLAUDE.md`, agents, permissions, and git context.
4. **Leaving a project** — if the agent `cd`s to a directory outside any git repo, the session returns to **home mode**.

### Home mode vs project mode

| Aspect | Home mode | Project mode |
|:-------|:----------|:-------------|
| cwd | home path (default `~`) | anywhere within the git repo |
| CLAUDE.md | not loaded from home path | loaded from git root + parents |
| agents/ | global only (`~/.linggen/agents/`) | global + project (`{git_root}/agents/`) |
| Permissions | `~/.linggen/permissions.json` | `{git_root}/.linggen/permissions.json` |
| Git context | none | branch, status, recent commits |
| Sandbox | unrestricted (home path scope) | unrestricted (no tightening on project entry) |
| Memory | global (`~/.linggen/memory/`) | project-scoped (`~/.linggen/projects/{encoded}/memory/`) |

### Session metadata tracks working folder

The session's `cwd`, `project`, and `project_name` fields are updated whenever the working folder changes. These are persisted to `session.yaml` so the UI can display the current context and group sessions by project.

### What triggers detection

Any bash command execution — both agent-initiated and user-initiated (`! cd /path`). After the command runs, the backend reads the new cwd (already tracked via the `__LINGGEN_CWD__` sentinel) and walks up looking for `.git/`.

### Auto-registration

When a git root is detected for the first time, the project is automatically registered in the project store (same as if the user had manually added it). No manual project management needed.

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
| `mission_id` | creation | no | Originating mission (if creator is `mission`) |
| `agent_id` | creation | yes | Which agent runs in this session |
| `model` | creation | yes | Model override for this session |
| `cwd` | creation | yes | Current working folder. Starts at home path. Updated on `cd`. |
| `project` | runtime | yes | Detected git root path (if any). `null` in home mode. |
| `project_name` | runtime | yes | Display name — last segment of git root (e.g. `linggen`). `null` in home mode. |
| `messages` | runtime | append | Chat history (`messages.jsonl`) |

### What's dynamic (can change during chat)

- **Model** — user can switch models mid-conversation via `/model`
- **Agent** — user can switch agents mid-conversation via `/agent` (tools change accordingly)
- **System prompt** — rebuilt each turn from current agent + skill + environment + project instructions
- **Title** — can be renamed
- **Working folder** — changes when agent or user runs `cd`. Triggers project detection.
- **Project context** — loads/unloads CLAUDE.md, permissions, git info when entering/leaving a git repo.

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
[5]  Environment             — always (platform, cwd, date)
[6]  Project instructions    — when in project mode and CLAUDE.md exists at git root
[7]  Memory                  — global memory always; project memory when in project mode
[8]  Tool schemas            — when effective_tools is non-empty
[9]  Plan mode instructions  — when plan_mode is active
[10] Delegation targets      — when Task tool is in effective_tools
[11] Plan execution context  — when plan is approved/executing
```

Layers 5-7 are sensitive to the working folder. When the session transitions between home mode and project mode, the system prompt is rebuilt with the appropriate project instructions and memory.

A zero-tool session (e.g., game-table skill with `allowed-tools: []`) gets layers 1-7 only — a minimal, focused prompt for pure conversation.

## Effective tools and permissions

Session permission mode controls which tools are available. See `permission-spec.md` for the full model.

- **Session is the single source of truth** for permissions. No tool lists in agent specs or skill frontmatter control access.
- **Four modes** (chat/read/edit/admin) define the ceiling. Mode is path-scoped and persisted per-session.
- **Skills** that need elevated permissions declare a `permission` section in frontmatter — the user approves when invoking, and the grant is written to the session's `permission.json`.
- **Missions** set mode + locked flag at creation time.

Available tools depend on the session's current mode for the current path:

| Mode | Available tools |
|:-----|:---------------|
| chat | None |
| read | Read, Glob, Grep, WebSearch, capture_screenshot, plan tools, AskUser, read-class Bash |
| edit | Everything in read + Write, Edit, write-class Bash |
| admin | Everything |

## Storage

All sessions live in one flat directory:

```
~/.linggen/sessions/{session_id}/
  session.yaml      # SessionMeta (id, title, created_at, creator, skill, cwd, project, ...)
  messages.jsonl     # one ChatMsg per line, append-only
```

No per-project, per-skill, or per-mission session directories. The `creator`, `skill`, `project`, and `mission_id` fields in `session.yaml` provide the metadata for filtering and grouping. Agent-created (sub-agent) sessions are ephemeral and not persisted.

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
| Permission mode | Mission tier (locked) | Config default from `linggen.toml` (unlocked) |
| Locked flag | `true` | `false` — interactive prompts resume |
| System prompt | Rebuilt from `ling.md` | Same — but cache invalidated to pick up any config changes |
| Chat history | Mission messages | Preserved — user sees full mission context |
| Session files | Stay in place | No move |

See `permission-spec.md` for the full permission model and session promotion details.

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

- `POST /api/sessions` — create session (accepts `skill`, `title`). No project required — starts in home mode.
- `GET /api/sessions` — list all user sessions (optionally filter by project)
- `DELETE /api/sessions/:id` — delete a session
- `POST /api/chat` — send message to a session (creates session if needed)

## UI behavior

- **New chat** — clicking `+` immediately creates a home-mode session. No project picker dialog.
- **Chat header** — shows the current working folder or project name (read-only). Updated reactively when the agent `cd`s.
- **Session list** — sessions can be grouped/labeled by their `project_name` field. No manual project management in the sidebar.
- **Project cards** — removed from sidebar. Projects are auto-discovered, not manually managed.
