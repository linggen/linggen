---
type: spec
reader: Coding agent and users
guide: |
  Product specification — describe what the system should do and why.
  Keep it brief. Aim to guide design and implementation, not document code.
  Avoid implementation details like function signatures, variable types, or code snippets.
---

# Tools

Syscall interface: built-in tools, safety rules, and the two-tier model.

## Related docs

- `session-spec.md`: effective tools, capability model.
- `skill-spec.md`: dynamic extensions (skill tools).
- `agentic-loop.md`: how tools are dispatched in the loop.
- `agent-spec.md`: agent tool declarations.

## Two-tier model

| Tier | What | Why built-in |
|:-----|:-----|:-------------|
| **Built-in tools** | Core coding ops | Need safety enforcement, internal state, or performance |
| **Skills** | Everything else | Extensible, replaceable, no code changes |

Built-in tools are the kernel API. Skills are userspace.

## Built-in tools

| Tool | Args | Purpose |
|:-----|:-----|:--------|
| `Glob` | `globs?, max_results?` | File pattern matching |
| `Read` | `path, max_bytes?, line_range?` | Read file contents |
| `Grep` | `query, globs?, max_results?` | Content search (ripgrep) |
| `Write` | `path, content` | Create/overwrite file |
| `Edit` | `path, old_string, new_string, replace_all?` | String replacement in file |
| `Bash` | `cmd, timeout_ms?` | Shell command execution |
| `GenerateImage` | `prompt, shape?, name?, reference?, seed?` | Draw one picture when the person asks for a drawing (never to show a real thing — that is a WebFetch photo). Kept with the session and numbered (`[turn3image1]`) like any picture an answer shows; in a skill also saved as `<skill>/data/pictures/<name>.png` |
| `lock_paths` | `globs, ttl_ms?` | Acquire file locks (multi-agent) |
| `unlock_paths` | `tokens` | Release file locks |
| `Task` | `target_agent_id, task` | Spawn subagent |
| `WebSearch` | `query, max_results?` | Web search (Tavily — your key from Settings first, else Linggen Cloud with sign-in; a failing key is reported, never swapped). Text only: results come numbered (`[turn2search1]`; chat-spec § Sources and pictures). Unreachable → the error says WebFetch still opens a known page |
| `WebFetch` | `url, max_bytes?` | Fetch URL content as text; the page and its pictures (`og:image` with a declared product's name and price, then `<figure>`/`<img>` with captions) come numbered (`[turn2image1]`) — the route to showing what something looks like |
| `Skill` | `skill, args?` | Invoke a skill by name |
| `RunApp` | `skill, args?` | Launch an app-enabled skill |
| `AskUser` | `questions` | Ask user structured questions mid-execution |
| `sense` | — | The room: presence, how busy the day is, the hour, and this machine's world block |
| `recent_activity` | `limit?` | What changed on this machine lately, and who did it (perception-spec.md §3) |
| `EnterPlanMode` | `reason?` | Enter plan mode (research-only) |
| `ExitPlanMode` | `plan_text` | Exit plan mode with completed plan |
| `UpdatePlan` | `plan_text?, items?` | Update plan progress during execution |

**Offered only where it runs.** A built-in may say it cannot run here
(`Tool::offered`); such a tool is left out of every tool list, so the model
never reaches for it. Instead it states one line of fact — what it does,
what it needs, what this machine lacks — and the prompt lists it under
`# Not available here`, so the model can tell the person how to get it.
Only tools the session's agent would otherwise have are listed (its tool
list, a skill's `allowed-tools`, less what the session withholds); a tool
left out by policy (plan mode, permission mode) is not. The model writes
the sentence; the engine gives the fact. `GenerateImage` is offered when
something can draw, else its line reads:

```
- GenerateImage: draws pictures; needs a ChatGPT sign-in, or an Apple Silicon Mac with 16 GB of memory and 8 GB free disk. Here: not signed in to ChatGPT, and the picture lane needs 16 GB of memory, this Mac has 8 GB
```

(the numbers are the `pictures` lane's; the tail is the lane gate's own
reason). The sources, in order:

- **ChatGPT** first, when the person is signed in to ChatGPT: the Responses
  API's `image_generation` tool through the default built-in model (Luna),
  as Codex CLI draws. About 30–40 s, a 1–2 MB PNG; it asks `square`
  1024×1024 or `landscape` 1536×1024, though the backend may pick its own
  (1312×1199 seen live). The plan's picture allowance is small: a refusal (a usage
  limit, a declined prompt) is the tool's result in one line, and nothing
  is drawn elsewhere instead.
- **This Mac** otherwise, when it passes the `pictures` lane gate (Apple
  Silicon, 16 GB unified memory): local FLUX. The lane downloads in the
  background after install, at the first quiet moment (storage-spec.md
  § Large downloads); a call that beats it starts the install at once
  (venv + ~4.3 GB model) and answers in one line; every runtime progress
  payload carries each lane's verdict. Renders run one at a time, one
  process per picture, `square` 512×512 or `landscape` 768×512.

The picture is kept in the session's `images/` and numbered in its refs
table at once (like a screenshot), so the chat shows it by number and it
goes with the session; the model never writes its address. In a skill the
skill also gets its copy (`name`, else a fresh name) and the result names
its `/apps/<skill>/…` URL for the skill's page; a `reference` (a file inside
the skill) keeps its pose and shape. Read tier: it writes no workspace file.

**Aliases**: `Read`/`Write`/`Edit` accept `file`/`filepath` for `path`. `Edit` accepts `old`/`search`/`from` for `old_string`, `new`/`replace`/`to` for `new_string`.

Tool names follow Claude Code convention (capitalized).

## AskUser

Lets the agent pause mid-loop to ask the user structured questions. Aligned with Claude Code's `AskUserQuestion`.

- **1-4 questions** per call, each with 2-6 selectable options.
- User can always type custom text ("Other").
- Blocks the agent loop until user responds (5 min timeout).
- Not available in delegated sub-agents — returns error.
- Not available in CLI mode.
- UI renders an inline card with option buttons in the chat stream.
- Cancelling the agent run unblocks the tool gracefully.

**Flow**: tool emits `AskUser` event → UI renders card → user submits via `POST /api/ask-user-response` → oneshot channel delivers answer → loop continues.

**Implementation**: `engine/tools.rs` (execution), `server/chat_api.rs` (response endpoint), `ui/src/components/AskUserCard.tsx` (UI).

## RunApp

Launches an app-enabled skill. The skill must have an `app` section in its frontmatter with a `launcher` type. See `skill-spec.md` → App skills.

- **`web`**: serves the skill directory as static files at `/apps/{skill-name}/`, emits `AppLaunched` event, UI opens an iframe panel.
- **`bash`**: executes the entry script in the skill directory, returns stdout/stderr.
- **`url`**: emits `AppLaunched` event with the external URL, UI opens in panel or new tab.

**Direct invocation**: when a user invokes an app skill via `/skill-name`, the server short-circuits the model — executes the app directly without entering the agent loop.

**Model invocation**: any agent can call `RunApp` to launch an app during a conversation (e.g., "let me open the dashboard for you").

**event**: `AppLaunched { skill, launcher, url, title, width?, height? }` — tells the UI to open the app panel.

**Implementation**: `engine/tools/delegation.rs` (`run_app()`), `server/chat_api.rs` (direct dispatch), `server/mod.rs` (static serving at `/apps/`), `ui/src/components/AppPanel.tsx` (UI).

## Tool dispatch

Dispatch order in `ToolRegistry.execute()`:

1. **Builtins** — canonical name match.
2. **Skill tools** — HashMap lookup.
3. **Unknown** — error.

**Implementation**: `engine/tool_registry.rs`, `engine/tools.rs`

## Access control

The session's effective permission mode for the target path controls which tools are available. See `permission-spec.md` for the full model.

- **Four modes**: chat (no tools), read, edit, admin — each a ceiling on what the agent can do for a path.
- **Path-scoped**: permissions are stored as `(path, mode)` grants and recomputed when cwd changes.
- **Deny/ask rules**: configured in `linggen.toml` to hard-block or force-prompt specific commands.
- Write-safety mode: checks that file was Read before Write/Edit.
- Redundancy detection: cache + loop-breaker for repeated calls — pure workspace reads only (`Read`, `Grep`, `Glob`). Every other tool, a skill's or an MCP server's included, runs on every call: a cached answer is a call that never ran.

**Flow**: permission gate in `handle_tool_action()` → classify action tier → check deny/ask rules → resolve target path → compare action tier to effective path mode → emit `AskUser` event if interactive, otherwise return permission-needed.

**Implementation**: `engine/permission.rs`, `engine/tool_exec.rs`, `ui/src/components/ToolPermissionCard.tsx`.

## Call deadlines

Each tool declares a wall-clock ceiling via `Tool::max_duration()` — a backstop against hangs, not a latency policy.

- Default 300s. Tree walks (`Glob`, `Grep`) 60s.
- `None` = unbounded, for work that is open-ended by nature: `AskUser` (waits on a person), `Task` (a subagent), `Bash`, `Skill`, `RunApp`.
- Unknown/skill-provided tools are unbounded — a skill declares its own work and the engine cannot guess its cost.
- On expiry the call is abandoned and the model receives a `timeout:` tool error, so the turn continues instead of stalling. A thread already blocked inside a syscall runs until the kernel returns, but it no longer owns the run.

Without a ceiling a blocking tool owns the run indefinitely: the turn never ends, the session stays busy, and cancellation cannot help because the block is inside a syscall that does not accept it.

**Implementation**: `engine/tools/builtin.rs` (`max_duration`, `tool_max_duration`), `engine/tool_exec.rs`.

## File safety

- All paths sanitized to workspace root.
- Parent traversal (`..`) rejected.
- Absolute paths outside workspace rejected.
- File listing/search use ignore-aware walking.

## Bash safety

- Commands executed via `sh -c` in workspace root.
- Timeout enforced (default 30s).
- Output captured (stdout + stderr).
- Workspace-scoped: working directory set to workspace root.
