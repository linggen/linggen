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
| `Read` | `path, max_bytes?, line_range?` | Read file contents. A picture file (PNG, JPEG, GIF, WebP; HEIC converted with `sips` on macOS, else said so), known by its first bytes, comes back as a picture: numbered (`[turn3image1]`, caption = file name) and kept in the session's `images/` like a screenshot; a model that sees gets it attached once in that turn, scaled like a pasted picture (long edge 1568 px; JPEG q0.85, or PNG when smaller; a small one as it came), a blind one gets the facts (format, width×height, size). Over 20 MB, undecodable or unconvertible: one line of facts. Path rules are Read's own |
| `Grep` | `query, path?, globs?, max_results?` | Regex search in a file or folder, absolute or relative to the session folder. Defaults to the session folder. Uses Read permissions for the search root and each file. With `path`, matches report absolute paths and globs are relative to the search folder (or the file's parent). |
| `Write` | `path, content` | Create or overwrite a file; if it changed since this session last read, wrote, or edited it, Read again before writing |
| `Edit` | `path, old_string, new_string, replace_all?` | String replacement in file |
| `Bash` | `cmd, timeout_ms?, run_in_background?` | Shell command execution. `run_in_background` starts it beside the chat and returns its task id at once; its end comes back as a turn (`background-tasks-spec.md`) |
| `GenerateImage` | `prompt, shape?, name?, reference?, seed?, model?` | Draw one picture when the person asks for a drawing (never to show a real thing — that is an ImageSearch or WebFetch photo). Kept with the session and numbered (`[turn3image1]`) like any picture an answer shows; in a skill also saved as `<skill>/data/pictures/<name>.png` |
| `lock_paths` | `globs, ttl_ms?` | Acquire file locks (multi-agent) |
| `unlock_paths` | `tokens` | Release file locks |
| `Task` | `target_agent_id, task, run_in_background?` | Spawn subagent. `run_in_background` returns at once; the subagent's end comes back as a turn |
| `TaskList` | — | The background tasks the caller started (the session's agent: any of its turns'; a subagent: its own) |
| `TaskOutput` | `task_id, lines?` | One of the caller's background tasks: status, how it ended, its output's last lines, a subagent's full result |
| `TaskStop` | `task_id` | Stop one of the caller's background tasks; its end is told as a turn |
| `SendMessage` | `to, message` | A message to a running subagent the caller started (task id or run id), or to its own parent run; read at the receiver's next step. No pause: stopping is `TaskStop` |
| `WebSearch` | `query, max_results?` | Web search (Tavily — your key from Settings first, else Linggen Cloud with sign-in; a failing key is reported, never swapped). Text only: results come numbered (`[turn2search1]`; chat-spec § Sources and pictures). Unreachable → the error says WebFetch still opens a known page |
| `ImageSearch` | `query` | Real pictures of a thing: one search on WebSearch's path, then the top 5 non-video result pages about the query read at once (5 s each, slow ones left out), up to 3 pictures a page whose own words touch the query and 10 in all, each numbered (`[turn3image1]`) with its caption and the page it is on. A model that sees also gets them as one labelled contact sheet. Run beside WebSearch, never inside it |
| `VideoSearch` | `query` | YouTube videos: one search on WebSearch's path narrowed by `site:youtube.com`; title, channel (oEmbed) and length (watch page) within 4 s, each numbered (`[turn4video1]`); the answer shows a card that plays in place |
| `WebFetch` | `url, max_bytes?` | Fetch URL content as text; the page and its pictures (`<figure>`/`<img>` with their own captions, then the `og:image` cover with a declared product's name and price; never the page title as a caption) come numbered (`[turn2image1]`) — the route to showing what something looks like |
| `Skill` | `skill, args?` | Invoke a skill by name |
| `RunApp` | `skill, args?` | Launch an app-enabled skill |
| `AppTool` | `app?, tool?, args?` | Do what an app offers from a session that is not the app's own. `app` alone lists that app's actions (tool, class, description, args); `app` + `tool` + `args` runs one. What is offered is what the app declares `reach: anywhere` (`skill-spec.md` § Reach), Yinyue's own `pet: true` reads besides. Judged as the action it names (`permission-spec.md` § App actions). Not offered to an `agent_run` delegate, a mission, or the lead of an app's own session |
| `browser_open` | `op, url` | Load a page in the one visible Chrome tab Linggen controls: `op=navigate` or `op=tab_open` (§ Browser tools) |
| `browser_read` | `op, filter?, region?, limit?` | Read that tab: `op=page` (tree with refs), `screenshot`, `console`, `tabs` |
| `browser_act` | `op, ref?, coordinate?, text?, keys?, …` | Act in it: `click`, `type`, `key`, `scroll`, `wait`, `tab_switch`, `tab_close` |
| `AskUser` | `questions` | Ask user structured questions mid-execution |
| `sense` | — | The room: presence, how busy the day is, the hour, and this machine's world block |
| `recent_activity` | `since?`, `until?`, `app?`, `verb?`, `text?`, `limit?`, `totals?` | This Mac's activity log (kept 400 days): what changed, who did it, stretches of use; `totals` sums counts and durations per app and verb for the span. JSON (perception-spec.md §3) |
| `note_write` | `text`, `look_again?`, `id?` | A note of the agent's own (at most 200 characters; at most 8 open, one more is refused with the open ones listed), shown to it beside every turn until finished. `look_again` wakes it once at that local time: `+40m`, `+3h`, `+2d`, `HH:MM` (next time the clock reads it), `tomorrow HH:MM`, `YYYY-MM-DD HH:MM`; a past time is refused. With `id`, rewrites that note (text and time replaced). A time is Yinyue's alone: on any other agent's note it is refused. Listed-only (below) |
| `note_done` | `id` | Finish one of the agent's notes: no longer shown, wakes no one. Listed-only |
| `EnterPlanMode` | `reason?` | Enter plan mode (research-only) |
| `ExitPlanMode` | `plan_text` | Exit plan mode with completed plan |
| `UpdatePlan` | `plan_text?, items?` | Update plan progress during execution |

**Listed-only tools.** `Express` (the pet's avatar) and the note tools
(`note_write`, `note_done`) are never part of the `*` wildcard: only an
agent whose tool list names them has them, and a call from any other is
refused at dispatch. Notes belong to the agent that wrote them
(`~/.linggen/notes/<agent>.json`); an agent whose own spec lists the note
tools keeps them at every table, with an explicit tool set (a skill's, or
what it names for that member) or an unrestricted one (`*`): the wildcard
holds them for the agent whose spec lists them alongside it, and for no
other. Yinyue lists them; Ling lists them beside `*`. A stranger's session
(a proxy room) has none. Two tools, not one with an op:
their params differ (app-action-spec § one tool per verb). Reading needs
no tool — the open notes ride beside each turn (session-spec § Beside the
turn).

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
reason). `model` selects `auto` (the default), `chatgpt`, or `local`.
Auto picks the first available source below. An explicit choice requires that
source to be available and never falls back to another; an unknown value is
an error. The sources, in order:

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

## AppTool

A session that is not an app's own does not carry the app's tools. Where `AppTool` is offered, the prompt carries a short index instead — `## Apps you can reach`, one line per installed app that has an action declared `reach: anywhere`: its name, its description cut to one line, its actions' names; nothing per action beyond the name. With no such app the section is absent. The index is built in one place beside the skills listing (`engine/prompt/app_index.rs`) and shown only where `AppTool` is offered (not in an app's own session, a mission, a delegate, a quiet turn of Yinyue's), for the apps this session's person may use and the session does not keep out.

So the tool schemas a session is sent do not grow when an app is installed: a plain Ling session is sent the same tools with six apps as with seven (a system test holds it), and only the index grows by a line. `AppTool` with no `tool` and no `app` lists every reachable app's actions; the index is what spares the model that call.

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

**Where a launch lands**: `app_launched` is broadcast — it reaches every surface, not only peers watching the launching session. Where it opens is decided in one place, `openApp` in `ui/src/lib/appHost.ts`: inside the desktop app (`inShell()`) that app's tab comes forward and the window comes up (show, unminimize, focus — the only shell IPC its page is granted); in a browser the web UI opens it in a new tab, and only for a launch from the session it shows. The pet and skill embeds ignore it. Inside the desktop app nothing falls back to a browser.

**From a skill script**: `POST /api/apps/launch {skill}` sends the same event (no session) for a script with no model turn to call `RunApp` from — e.g. DJ's play asked from the phone. The page itself is how the caller learns it came up.

**Implementation**: `engine/tools/delegation.rs` (`run_app()`), `server/chat_api.rs` (direct dispatch), `server/mod.rs` (static serving at `/apps/`), `ui/src/lib/appHost.ts` `openApp` (UI: where it opens), `ui/src/apps/AppTabs.tsx` (the desktop app's tabs).

## Browser tools

A session is offered three browser tools, `browser_open`, `browser_read` and `browser_act`, each with an `op` — the same three, with the same ops, arguments and texts, the MCP door publishes (`mcp-spec.md`). They are defined once, in `engine/tools/browser_def.rs`; the door's catalogue and the session's tools both read it, and a lock fails when what a session is offered differs from what the door publishes. A plain session is sent three schemas where it was sent ten (`Browser_navigate` … `Browser_tabs`).

- **Same behaviour behind the three.** Every op keeps what it did: the Read tier, the extension's own Allow prompt for an action that changes a page, the place rules (a browser call keeps what is said in its turn's place), no cache, the wall-clock ceiling, and the bridge's progress line under the running tool. An op's time limit is the definition's, the door's: the tab ops that never ask the person (`tabs`, `tab_switch`, `tab_close`) wait up to 150 s like the others of the tab (a session waited 20 s before).
- **A call is checked as the door checks it**, before the bridge is asked: an unknown field, a missing `url`, a non-absolute URL, a click with neither `ref` nor `coordinate` is refused in the words the door uses. A model that calls an old name or leaves out `op` is told the form that works.
- **Old names in a declaration.** The ten old names are never offered, but a skill's `allowed-tools`, an agent's `tools:`, a mission's list or `place.*.tools` that still names one selects the tool that now holds it, in the offered set and at the call gate (and a withheld old name withholds that tool). One function resolves it (`canonical_tool_names`): a name selects the tool of that name, or every tool that lists it as an alias; `Browser_tabs`, whose ops moved into all three tools, selects all three. The delegate's fixed list holds the three.
- **Plan mode** does not offer them (its list is read-only research); they are not cacheable.

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
- `timeout_ms` limits the foreground wait (default 120,000 ms, capped at 600,000 ms). At the limit, a running command moves to a session background task with its output so far and reports its end in a new turn. Without a session to keep it, Bash kills the command and says why.
- Output captured (stdout + stderr).
- Workspace-scoped: working directory set to workspace root.
