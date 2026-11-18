---
type: spec
reader: Coding agent and users
guide: |
  Product specification — describe what the system should do and why.
  Keep it brief. Aim to guide design and implementation, not document code.
  Avoid implementation details like function signatures, variable types, or code snippets.
---

# Session & Context

A session is the fundamental unit of interaction. Each session holds one context — an isolated conversation with its own tools, permissions and message history — and one or more agents at its table, each with its own persona and model (`shared-session-spec.md`).

## Related docs

- `chat-spec.md`: events, rendering, message model.
- `agent-spec.md`: agent definitions, delegation.
- `skill-spec.md`: skill format, activation.
- `agentic-loop.md`: how the engine runs inside a context.
- `shared-session-spec.md`: several agents at one table — members, the thread, who answers.

## Execution runtime

The existing New chat button creates a native Linggen session. A second
**New Claude session** button creates a conversation with one Claude Agent.
All messages in it go directly to the Claude Agent SDK; no `@` is needed and
native agents cannot join. The runtime stays fixed for that session.

Claude sessions use SDK tools, context and permissions. Linggen's widgets
render their output and user interactions, but native persona, skill/mission
injection, compaction and path authorization are not applied. The SDK owns
its resume state independently of the visible chat log. A linked parent may
start and track the task without entering its conversation.

A folder's own Claude configuration (`.claude/`, `CLAUDE.md`,
`CLAUDE.local.md`, `.mcp.json`, in the folder or a parent short of `$HOME`)
loads only after the person trusted that folder once, as Claude Code asks.
A folder with none is never asked about: it runs on user settings until some
appears. A "no" holds for that session and folder — kept in the session's
directory, so an engine restart does not ask again — until Clear or an SDK
reset ends the conversation.

## Core concept

Session = Context. One session, one context. They are the same thing.

A session is created by a **creator**, which determines the session's initial configuration — tools, system prompt, model. The creator copies its spec into the session. After creation, the session's configuration can change dynamically during the conversation.

## Working folder

Every session has a **working folder** (cwd) — the directory the agent operates in. The working folder replaces the old "project selection" concept.

### Default workspace

A new chat works in the **default workspace**, `~/.linggen/workspace` (under `$LINGGEN_HOME` when set; made when missing). Not `~/.linggen` itself — credentials and config live there — and not the session's own folder, where the engine keeps its files.

A session's working folder is fixed **when it is made**, on the server, and never empty. It is another folder only when one was chosen for that session: a picked project (`project_root` at create), a skill's `cwd:`, a mission's `cwd:`, or a `cd` inside the session. A folder chosen for one session never carries to the next New chat. What a surface sends with later calls (a chat turn's or a `!` command's `project_root`) never moves an existing session — the session's own `cwd` wins; only a `cd` moves it.

The **Linggen language** (`language`, Settings → General) is the language Linggen writes in for the person: a BCP-47 tag, unset = Auto (the Mac's first preferred language, then `en`). One resolver (`src/language.rs`) answers it for the prompt's language line, `LINGGEN_LANGUAGE` in skill commands, and `GET /api/user/language`.

**Skill-bound sessions** start in the skill's declared `cwd:` (e.g. `cwd: ~/.linggen` for `ling-mem`), else the skill's own folder. Resolution order at `POST /api/sessions`: skill's `cwd:` → request's `project_root` → the skill's folder (skill sessions) or the default workspace. A session first made by `POST /api/chat` follows the same rule; a mission's run session takes the mission's `cwd:`, else the default workspace.

A `session.yaml` written before this rule with no `cwd` is read as its project, else its skill's folder (skill sessions), else the default workspace, and keeps that from its next write — nothing downstream (memory scope, Bash, permissions) sees a session without a folder.

### How it works

1. **New chat** — starts in the default workspace. No project picker — just click `+` and go.
2. **Agent or user runs `cd`** — after any bash command, the backend checks the new cwd.
3. **Git detection** — if `.git/` exists in the cwd or any parent up to `$HOME` (never above it; home itself is never a project), the session enters **project mode** for that git root. The engine loads the project's `CLAUDE.md`, agents, permissions, and git context.
4. **Leaving a project** — the project is sticky: a `cd` to a folder outside any git repo moves the working folder but keeps the last project (a shell bouncing out of the repo keeps it); a `cd` into another repo switches it. A session that has never been in a repo stays in **home mode**.
5. **Claude sessions** — the SDK reports its main thread's `cd` (a subagent's is ignored), and the session moves the same way. The SDK conversation still resumes in the folder it started in.

### Home mode vs project mode

| Aspect | Home mode | Project mode |
|:-------|:----------|:-------------|
| cwd | default workspace, or any non-git folder | anywhere within the git repo |
| CLAUDE.md | not loaded outside a git repo | loaded from git root + parents, up to `$HOME` |
| agents/ | global only (`~/.linggen/agents/`) | global + project (`{git_root}/agents/`) |
| Permissions | session `permission.json` path grants | session `permission.json` path grants |
| Git context | none | branch, status, recent commits |
| Sandbox | effective mode for cwd | effective mode for cwd |
| Memory | global (`~/.linggen/memory/`) | project-scoped (`~/.linggen/projects/{encoded}/memory/`) |

### Session metadata tracks working folder

The session's `cwd`, `project`, and `project_name` fields are updated whenever the working folder changes. These are persisted to `session.yaml` so the UI can display the current context and group sessions by project.

### What triggers detection

Any bash command execution — both agent-initiated and user-initiated (`! cd /path`). After the command runs, the backend reads the new cwd (already tracked via the `__LINGGEN_CWD__` sentinel) and walks up looking for `.git/`.

### Auto-registration

When a git root is detected for the first time, the project is automatically registered in the project store (same as if the user had manually added it). No manual project management needed.

### Context compaction

The thread is the session's, and so is its compaction: one summary row (pseudo-sender `compaction`), written into `messages.jsonl` where the verbatim tail begins and read by every member as a system note. The rows before it stay in the file for scrollback; no member's thread reaches past the newest summary. It runs before a turn when the thread outgrows the **smallest** member's context window × `compact_threshold`, and on `/compact`; the session's default member's model writes it. A member's window is read again when its model changes.

Inside one long turn the engine compacts the same way, and writes the result back into the file so the next turn does not reload what this turn just dropped:

1. **Evict** — replace old `tool_result` bodies with a placeholder in place (cheap, structure preserved).
2. **Summarize** — if still over budget, replace everything between the system prompt and a recent verbatim tail with one model-written structured summary, inserted as the `compaction` row above.
3. **Fit the tail** — a tool result kept verbatim that is itself larger than the window is shortened in the live tail (the row stays; the body is cut). The next turn rebuilds its thread from that summary, so it does not clone the span this turn dropped.

`compact_threshold` (fraction; per-session → `linggen.toml` → 0.95 default) and `compact_focus` (hint fed to the summary prompt) are per-session, set via `POST /api/chat/compact_config`, persisted in `session.yaml`, and re-read on every engine build — so they survive restart and apply to chat-only and mission sessions. `messages.jsonl` is the unbounded source of truth; there is no blunt history cap.

## Creators

Every session has a creator that determines how it was born and what capabilities it starts with.

| Creator | How it's created | Typical tools | Example |
|:--------|:-----------------|:-------------|:--------|
| `user` | User clicks "New Chat" in UI or sends first message | All agent tools (`["*"]`) | Normal coding session |
| `skill` | Skill app creates a session via API with `skill` field | Defined by skill's `allowed-tools` | Game-table creates a zero-tool session |
| `agent` | Sub-agent spawned via `Task` tool | Inherits parent's tools | Ling delegates exploration to itself |
| `mission` | Cron scheduler triggers a mission run | Mission agent's tools | Nightly code review mission |

### The session spec

Each occasion a session is made for is a **preset**: a person's chat, an
app's chat, Linggen's daily session, the companion's own thread, a hand-off,
a mission, an `agent_run` delegate, an app's one-off job (bound values or
tools withheld), a Claude session. A kept session's preset is read from what
its `session.yaml` already holds (creator, runtime, id shape, params,
withheld tools) — nothing new is stored. The preset's row in one table gives
the session's strategies: who leads, its folder, its tools, which memory,
how a turn is framed (verbatim or as a task), who answers beyond the grants
(the person, the caller, or nobody), how its thread is kept (its own,
the day's, rolling). A new kind of session is a new row.

**One kind of spec, many presets — not one session that holds everything.**
What is shared is the mechanism (one spec type, one writer). What differs is
the content: each occasion gets only what it needs — its tools, its part of
memory, who answers beyond the grants, whether its talk may leave. A person's
chat carries a one-line-per-skill menu (about 750 tokens for eight skills,
measured 2026-11-17) and loads an app's instructions only when it takes that
app up with the `Skill` tool (7–15 thousand tokens each); an app's own
session has no menu, has that app's instructions from the start, and keeps
only core and the app's own memory. A session with every app loaded would
carry about 75 thousand tokens of instructions on every turn, and could not
keep a mission unattended, a private app private, or a read-only device
read-only. A new occasion is a new preset row or a work tag, never a wider
default.

A session may store `work: code` to add coding guidance to its system prompt
and to let it run programs in its own folder without asking
(`permission-spec.md` § Coding sessions).
Chat, Delegate and Handoff take this tag; all other presets ignore it.
The guidance is in `prompts/work-code.toml`: core lines followed by the answering model family's lines when available.
The tag does not change tools or memory, and Task children inherit it. Only
a person (creating the session, or the Coding switch) or an `agent_run`
caller sets it; no tool does.

Create an independent coding chat with `POST /api/sessions` and `work: "code"`; a normal request without it is unchanged. App and other unsupported presets refuse the switch. `PATCH /api/sessions` sets `work: "code"` or clears it with `work: "none"`; it takes effect on the next turn, except that clearing it stops programs running unasked at the next permission check.

A new `agent_run` session defaults to coding guidance when given `write_dir`, otherwise none; `work: "code"` or `"none"` overrides that default. A continued run ignores `work`. Coding sessions never join the daily session.

**Preset grants.** A spec carries what its session may do without asking,
set by whoever makes it: a skill's declared paths when it is taken up in a
person's session, a mission's and its skill's, an `agent_run` delegate's
posture with the caller's `grants` (`mcp-spec.md` § agent_* group) — its
folders. Programs need no grant: a coding session runs them where it may
edit (`permission-spec.md` § Coding sessions).
One function in the spec writes them into the session's grant
table, in order (a later path can narrow an earlier one); a skill's leaves a
path already at its level as it is, so what the person set under it stays.
A test holds that no other code writes grants, besides a person's own
answers and the listed tests. A session made from its meta has none.

The spec is the only writer of a session's prompt profile, tool
restriction, interactivity, world and framing; a turn's place narrows the
turn on top of it and puts it back when the turn ends (§ A turn's place). A
person typing into a session runs it under their policy, verbatim, with the
memory of whom the turn belongs to; in an `agent_run` delegate's session the
delegate's toolset stays and its caller still answers beyond the grants.

**A turn's ceiling.** A turn a paired device sent below the owner's level
runs at no more than that level: `SessionSpec::typed` takes the level the
request was judged at (`x-linggen-level`, never the body) as the spec's
`ceiling`, and the permission check refuses — never asks for — any action
above it, whatever the session's grants. At read a turn may look, not
change; at chat it uses no tool that needs more; at edit it writes but runs
nothing admin. Such a turn acts within the session's grants and asks no
one, carries none of the owner's memory (the store is the owner's), and
runs no MCP tool ungated — each is judged by its tier under the ceiling.
A `Task` child keeps its parent turn's ceiling. The owner's turns, and this
machine's, have none, so a session the owner started keeps its grants for
the owner's turns. A session another runtime hosts (Claude) and the
companion's own thread take no turn below admin.

### Creator inheritance

When a session is created, it inherits configuration from its creator:

1. **Members** — an ordinary chat seats `ling`; Yinyue's daily thread seats her; a skill seats its declared `members:`; a mission its agent. The first member is the **lead**.
2. **Tools** — the session's, set by its lead: the lead agent's tool list (e.g., `ling` has `["*"]`), or a bound skill's `allowed-tools`. A member who joins later uses that set; a skill may name a member's own (`place.<agent>.tools`).
   A session may also **withhold** tools from every member, whatever the set: `withheld_tools` in `POST /api/sessions` (kept in `session.yaml`) — a tool name or a whole MCP server (`mcp__memory`). Withheld tools are never offered, are refused if called, and stay withheld in a run delegated from the session. The engine's live suite makes its scratch sessions with `[mcp__memory]`, so a test turn cannot write the real memory store.
3. **System prompt** — per member: its soul (personality + body) + voice + place (`## Where you are`) + skill body (the lead's, when bound)
4. **Model** — per member: the session's override for it, else its agent config / spec (the companion's `pet.model`), else the default routing chain

## Session state

Each session carries:

| Field | Set at | Mutable | Purpose |
|:------|:-------|:--------|:--------|
| `id` | creation | no | Unique identifier (`sess-{timestamp}-{uuid8}`) |
| `title` | creation | yes | Human-readable name, can be auto-generated |
| `created_at` | creation | no | Unix timestamp |
| `creator` | creation | promotion only | Who owns this session: `user`, `skill`, `agent`, `mission`. Changes from `mission` → `user` on promotion. |
| `skill` | creation | no | Bound skill name (if any) — the app the session was made for, and the place of its turns (§ A turn's place) |
| `params` | creation | no | The values the session is bound to, by the bound skill's declared session params (`skill-spec.md` § Session parameters) — e.g. `{save: test}`. Checked against the declaration when the session is made and on every use. Absent: the skill as it runs unbound. |
| `mission_id` | creation | no | Originating mission (if creator is `mission`) |
| `agents` | creation | grows | The members `[{id, model}]`, the lead first; `@name` seats one for good. Old sessions' `agent_id`/`model_id` migrate to a one-member list. |
| `cwd` | creation | yes | Current working folder — never empty. Set at creation (see § Default workspace). Updated on `cd`. |
| `project` | runtime | yes | The last git root the working folder was in; kept when the folder leaves every repo. `null` until the session is first in a repo. |
| `project_name` | runtime | yes | Display name — last segment of git root (e.g. `linggen`). `null` with `project`. |
| `messages` | runtime | append | Chat history (`messages.jsonl`) |

### What's dynamic (can change during chat)

- **Model** — per member; the picker sets the model of the member the chat speaks with
- **Members** — `@name` addresses an agent and seats it if absent
- **System prompt** — rebuilt each turn from current agent + skill + environment + project instructions
- **Title** — can be renamed
- **Working folder** — changes when agent or user runs `cd`. Triggers project detection.
- **Project context** — loads/unloads CLAUDE.md and git info when entering/leaving a git repo. Permissions are still session-scoped and recomputed from path grants.

### What's fixed for the session lifetime

- **Bound skill** — set at creation, cannot be changed. Start a new session for a different skill.
- **Bound params** — one session, one value per param: a different save is a different session.
- **Creator** — immutable except for promotion (mission → user when a human takes over).

## A turn's place

What an app gives a turn belongs to the turn, not the session. A turn's
**place** is the skill of the page it came from — or none, **home**: Yinyue's
own surfaces, a plain chat — with the values the session is bound to for that
skill (§ Session parameters in `skill-spec.md`; another skill's turn in the
session sees none).

- **Which place.** A chat request takes the skill its page names, else the
  session's own. A turn no page sent — Yinyue's own turns, her turn at an
  app's table, a light turn, a look-again wake, a relayed message, a
  background task's end, an approved plan's run — stands in the session's own
  place. A message resent after a restart stands where it was sent. A delegated run stands where its parent's turn stands. A mission
  run and an `agent_run` session have no skill: home.
- **Set and cleared.** When the turn starts, its place sets the skill's
  SKILL.md in the prompt, its tools (`allowed-tools`; `place.<agent>.tools`
  for a member who isn't the lead; the skill's own tools), its memory scope
  (core and the app's own memory), the values its commands read, and — for a
  place other than the session's own — its folder (the skill's `cwd`, else its
  folder; home's is the default workspace). When the turn ends all of it is
  put back: the next turn, in whatever place, sees nothing of this one. A
  skill taken up mid-turn (`/name`, the Skill tool) is not the place: its
  tools stay for later turns as before. In a day's session a private app
  is never taken up, as the place or mid-turn (`one-ling-spec.md` § Linggen's daily session). A `cd` in another app's place moves
  only that place's folder, never the session's.
- **Grants per place.** Each place keeps its own grants in the session's
  `permission.json`: a grant given in one app is not honoured in another
  (`permission-spec.md` § Session grants).
- **Queue.** A message sent while a turn runs meets it by the places of both:
  in the same place, the skill's `queue` rule (`skill-spec.md` § Queue); in
  different places the message waits for the running turn to end, and is
  never folded in or steered into it.
- **Rows.** Each row a turn saves to `messages.jsonl` carries its place
  (`place`: the skill's name, or `:home`), stamped as it is saved; in a
  day's session a plan's rows name the place of the plan's row. A row
  without a place — every row written before places, and rows saved while
  no place is kept (a mission or `agent_run` run, a page's own writes) — is
  the session's own place.

With `agent.daily_session` off, a page names the skill its session was
made for, so every turn of a session shares one place: the session's. On,
an app's chat is the day's session (`sess-day-YYYY-MM-DD`, from 04:00
local; `one-ling-spec.md` § Linggen's daily session): a page's turn stands in its
app, other turns at home. The day's session is made by `agent` (as Yinyue's
own thread; the memory scan reads only `user` chats), Ling its lead. A
private app (`private-talk: true`) never joins it: its chat keeps a session
of its own, and from its turns no tool carries words out (`skill-spec.md`
§ Private talk).

## System prompt assembly

The system prompt is rebuilt each turn from layers. Which layers are included depends on the session's current state:

```
[1]  Agent personality       — always (sets response style)
[2]  Agent body              — always (sets identity and behavior)
[3]  Available skills        — skill names + descriptions for discovery; none in a skill-bound session (app or not), whose skill is its place
[4]  Active skill body       — when session has a bound skill
[5]  Environment             — always (platform, cwd, date; this machine is the person's own and Ling runs on it — its live state is a shell command away)
[5a] Not available here      — owner sessions: tools the agent would have but this machine can't run, one fact line each (tool-spec.md § Offered only where it runs); stable prefix, changes on sign-in or install
[6]  Project instructions    — when in project mode and CLAUDE.md exists at git root
[7]  Memory                  — global memory always; project memory when in project mode
[8]  Tool schemas            — when effective_tools is non-empty
[9]  Plan mode instructions  — when plan_mode is active
[10] Delegation targets      — when Task tool is in effective_tools
[11] Plan execution context  — when plan is approved/executing
```

### Beside the turn

Three notes ride outside the system prompt, so a changed reading never moves
the cached prefix:

- **The world** (`perception-spec.md` §2) — every native turn of a session made
  by `user`, `skill` or `agent`, whatever its tool list (`["*"]` included);
  never `mission` or `external`, a `Task` child, or a proxy-room consumer
  (`prompt::world::world_rides`, from the stored `creator`). Built once when the
  turn starts and **kept in the thread** right after the turn's words: every
  model request of the turn sends the same bytes, and the next turn's request
  begins with this one's plus its reply. In `messages.jsonl` it is a row from
  the pseudo-sender `world`: read back as a system note by every member, never
  listed by the history views, never spoken, not counted in a capped thread's
  live window, left out of compaction summaries. A turn whose readings match
  the last kept block's but for the clock keeps no new one.
- **The room** (presence, work, the hour) — a session whose tools name `sense`;
  built per request, just before the turn's words, never kept.
- **The agent's own notes** (`# Your notes`, yinyue-companion-spec § Notes) — a
  turn that offers `note_done`; built per request after the room, never kept:
  they are the speaking agent's alone, and a kept row is read by every member
  of a table. Nothing when it has no open notes.

The prompt export (`GET /api/chat/system-prompt`, "Copy System Prompt") shows
under `beside_turn` the room, the notes and the block the next turn would keep. Sessions
run by the Claude runtime are built elsewhere and get neither yet.

Layers 5-7 are sensitive to the working folder. When the session transitions between home mode and project mode, the system prompt is rebuilt with the appropriate project instructions and memory.

A zero-tool session (e.g., game-table skill with `allowed-tools: []`) gets layers 1-7 only — a minimal, focused prompt for pure conversation.

A **light turn** (`engine::light_turn` — the one line an agent gives after the companion speaks in its chat) is not a turn of this assembly: one model call with the soul, a short place (layer 2's place, else the skill's description), the language line, the thread's last few spoken lines and a hidden ask. No tool schemas; a `SILENT` reply keeps nothing.

## Effective tools and permissions

The session's effective permission mode for the current working folder controls which tools are available. See `permission-spec.md` for the full model.

- **Session is the single source of truth** for permissions. No tool lists in agent specs or skill frontmatter control access.
- **Four modes** (chat/read/edit/admin) define the ceiling. Mode is always path-scoped and persisted per-session as `(path, mode)`.
- **Current cwd determines the visible mode**. After `cd`, the backend recomputes the effective mode by longest matching path grant. No matching grant means `chat`.
- **Skills** that need elevated permissions declare a `permission` section in frontmatter — the user approves when invoking, and the grant is written to the session's `permission.json`, under the turn's place (§ A turn's place).
- **Missions** set mode + paths at creation time. If a mission needs more permission than its grants allow, it records permission-needed instead of silently widening access.

Available tools depend on the session's effective mode for the current path:

| Mode | Available tools |
|:-----|:---------------|
| chat | None |
| read | Read, Glob, Grep, WebSearch, WebFetch, plan tools, AskUser, read-class Bash |
| edit | Everything in read + Write, Edit, write-class Bash |
| admin | Everything in edit + admin-class Bash |

Reads are gated like writes — an unguarded path defaults to `chat` (no tools), so a `Read` outside any grant prompts the user just like an edit would. This is required for proxy-consumer safety (a consumer cannot reach `~/.ssh` from a session granted on `~/work`).

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
| Permission grants | Mission path grants | Existing grants preserved; user can change them |
| Permission-needed behavior | Run records pause/failure | Interactive prompts resume |
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

- `POST /api/sessions` — create session (accepts `skill`, `title`, `params`, `project_root`). No project required — starts in the default workspace. Returns `{id, cwd}`. `params` the skill doesn't declare, or values it refuses: 400 with the reason.
- `GET /api/skill-sessions?skill=` lists each session with its `params`.
- `PATCH /api/sessions` — rename / set the session model
- `DELETE /api/sessions/all` — delete a session (`session_id`)
- Session lists reach surfaces through `page_state`; skill pages use `GET /api/skill-sessions?skill=`.
- `POST /api/chat` — send message to a session (creates session if needed)

## UI behavior

- **New chat** — clicking `+` immediately creates a session in the default workspace; the request carries no folder. The page mirrors the open session's folder in memory only, so the folder of the chat open before never reaches the new one. No project picker dialog.
- **Chat header** — shows the current working folder or project name (read-only). Updated reactively when the agent `cd`s. A Coding switch beside the session controls adds or removes coding guidance for the next turn in a person's chat, delegate or hand-off; other sessions do not show it.
- **Session list** — sessions can be grouped/labeled by their `project_name` field. No manual project management in the sidebar.
- **Project cards** — removed from sidebar. Projects are auto-discovered, not manually managed.
