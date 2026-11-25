---
type: spec
reader: Coding agent and users
guide: |
  Product specification — describe what the system should do and why.
  Keep it brief. Aim to guide design and implementation, not document code.
status: 2.7 + memory v2 (in trial) — one door, fifteen tools (memory 5, agents 4, browser 3, apps 1, other agents' memory 1, Linggen's own notes 1) at /mcp; the two-server layout (24 + 17 tools) is retired
---

# Linggen MCP — the one door

One MCP server, on the engine, for **every** outside agent (Claude Code, Codex,
Cursor and others): `http://127.0.0.1:9527/mcp`, streamable HTTP, stateless
JSON-RPC (`initialize`, `ping`, `tools/list`, `tools/call`). Its tools come
from one catalogue (`src/server/mcp/catalog.rs`, composed with memory's
`crates/linggen-memory/src/catalog/`). The server is named `linggen`; in Claude
Code a tool is `mcp__plugin_linggen_linggen__<tool>`.

Thirteen tools, in list order:

| Group | Tools | Fronts |
|:------|:------|:-------|
| Memory (5) | `memory_search`, `memory_add`, `memory_update`, `memory_delete`, `memory_upkeep` | the store the engine hosts |
| Agents (4) | `agent_run`, `agent_status`, `agent_permit`, `agent_end` | Linggen's agents (delegate a task) |
| Browser (3) | `browser_open`, `browser_read`, `browser_act` | the linggen-browser extension (control module) |
| Apps (1) | `app` | what the person's apps do on this Mac (last, so every other tool keeps its place) |

A tool with several operations takes `op`. Every schema is strict
(`additionalProperties: false`): an unknown field, a field of another op, a
missing required field or a bad value is refused, and the refusal shows the
form that works.

Memory is part of Linggen, not a second product: dreaming, scanning and the
review queue run on the engine. That is why memory and the other capabilities
share one door.

## Views — one address, the view by caller

| Caller | Sees |
|:-------|:-----|
| this machine (loopback) | all fifteen tools; `app`, `memory_agents` and `memory_notes` answer the owner at this Mac itself, not a tunnelled device |
| an approved computer (token in `x-linggen-device`) | the five memory tools only; `memory_upkeep` without `dream_run` (twelve ops). Delegation and browser control are this machine's alone |
| a phone, or a caller with no token off this machine | refused (403 / 401 `PAIRING_REQUIRED`). Phones use `/api/memory/<verb>`, not MCP |

A tool or op outside the caller's view cannot be called (`unknown tool`, or
`op` not one of the listed). An approved computer's calls on `/mcp` are
recorded as that machine's presence (Settings → devices shows it seen).

The engine's own agents reach memory at `/mem/mcp` (internal, this machine
only: the five memory tools, eleven upkeep ops). It is not for hosts.

## Memory tools

- **`memory_search`** — `op` omitted or `semantic`: `query` (required), `tier`,
  `scope_root` (filled by the host), `limit`. `op: get`: `id`. `op: list`:
  `tier`, `types`, `scope_root`, `indexed`, `past_ttl`, `day`, `unjudged`,
  `sort`, `limit`, `offset`. A call with only `query` is a semantic search. An
  `id` or list filters without `op` are refused with the form that works.
- **`memory_add`** — `content` (required), `type`, `from`, `tier`, `scope`,
  `summary`, `indexed`, `global`, `replace_ids`, `user_directed`,
  `occurred_at`, and `host`, `source_session`, `cwd`, `root` (filled by the
  host).
- **`memory_update`** — `id` (required), `content`, `type`, `tier`, `scope`,
  `summary`, `indexed`, `global`, `user_directed`. It takes no `host`, `cwd`,
  `root` or `source_session`; a stamped one is refused.
- **`memory_delete`** — `id`.
- **`memory_upkeep`** — `op` is required: `session_start`, `harvest_day`,
  `days`, `remember_day`, `dream_lease`, `role`, `sweep`, `issues`,
  `issue_add`, `issue_resolve`, `chains`, `dream_status`, `dream_run`.
  `dream_status` is the engine's: undreamed days, first undreamed and first
  unscanned day, open review items, the in-flight run, the last run, its error
  and its `last_run_cannot_run`. `dream_run` starts the dream mission (quiet
  variant only, since MCP callers can't receive AskUser) and is this machine's
  alone. Hosts are steered to run the dream with their own model
  (`/linggen:dream`) and use `dream_run` only to offload to the engine's
  executor. The calling agent is the solver of the review queue
  (`issues`, `issue_resolve`): the queue holds facts and bookkeeping.

The memory protocol (three tiers, voice law, `replace_ids` / `user_directed`
guard, the status-replaces-status rule) lives in the server text and in the
plugin's memory skill; the memory part of the text needs no account.

## The app tool

`app` lets an outside agent do what the person's apps do on this Mac — "play
some jazz" in Claude Code plays it. It has two ops: `list` (no `app`: one line
per app that has an action an outside agent may use — its name, what it is
for, its actions' names; with `app`: that app's actions, each with its
class, description and arguments) and `call` (`app`, `action`, `args`: runs it
and returns its result; a command that exits non-zero is a tool error).

It is not a second list or a second judge. What it lists is the actions a
skill declares `reach: anywhere` (`skill-spec.md` § reach), as a session's
`AppTool` finds them, kept to the ones an outside agent may run: a
destructive, admin or unclassed action, or one the app's own grant does not
cover, is not listed, and an app left with none has no line. Every call is
judged by `skill::judge` as the
action it is, with the outside agent as one more asker (`permission-spec.md`
§ App actions): a `read` or `edit` action runs under the app's own grant on
its own folder; a `destructive` one is refused — "the person does this
themselves, in the app" — since an outside agent has no way to ask the person
here; an `admin` or unclassed one is refused. An action the app keeps to its
own session, or offers to Yinyue alone, is not offered.

For the owner at this Mac alone. A caller that is anything else — a tunnelled
device, a request carrying a token — is refused before an app is looked up,
and the answer does not say whether the app exists. An approved computer does
not see the tool. No file, shell or other built-in tool of Linggen's sessions
is at the door (a test holds the names).

An app that keeps its talk private (`private-talk: true`) is closed to outside
agents until the owner opens it: not listed, and a call is refused with a
reason that names the setting. The owner opens it in Settings → Tools & MCP →
Outside agents, one switch per private app that offers an action anywhere. The
list is the engine config's `outside_apps` (absent: none), set only through
`PUT /api/mcp/outside-apps` at this Mac; a saved config keeps what it holds.

## Other agents' memory (`memory_agents`) — memory v2, in trial

Claude Code and Codex keep their own memory their own way; Linggen only
**reads** it — never edits, moves or copies it — and forwards what each
agent remembers to the others. One tool, this machine's owner at this Mac only:

| op | does |
|:---|:-----|
| `mode` | `v1` or `v2`: how this Linggen serves outside hosts (`[memory] outside_agents` in the config; absent = `v2`; `v1` is the way back). The plugin's hooks ask it. |
| `index` | the other agents' index lines for a project (`cwd`), never the calling `host`'s own; nearest project first, then the person; at most 4 KB, each group tagged with its source |
| `read` | one note by absolute (or `~/`) path; only a note file inside a registered source, links and `..` resolved first |
| `search` | words across the registered sources, any language, best line first |

- The sources are one declared list (`outside_memory::sources`): Claude Code's
  `~/.claude/projects/<encoded project path>/memory/` and `~/.claude/CLAUDE.md`,
  and a Codex entry that reports "no data / format not known yet" until its
  memories are read. An unreadable or missing source is skipped and reported.
- Key-shaped strings are replaced before anything leaves.
- **v2** (the switch): the plugin's session-start hook injects the `index`
  slice instead of core memory; the per-turn recall, the capture reminder, the
  cwd stamp and the delegate recall do nothing; the server text drops the
  save rules. Linggen's own agents (Ling, Yinyue, Linggen's sessions) are
  unchanged in both modes. **v1** is today's behaviour. An approved computer
  sees the same as before in both.

## Linggen's own notes (`memory_notes`) — memory v2, in trial

Linggen's own memory (`memory-spec.md` § Memory V2), for a host that has none
of its own to keep: one tool, ops `index` (the notes for a project), `read`
(a note by id, or a topic file by path), `search` (words, also across other
agents' memory), `write` (add, update, void, no_raise by id). Served to the
owner at this Mac when `[memory] outside_agents = "v2"`; in v1 it answers
that it is not served. A host on the door is always a coding reader: notes
about body, mind and people, and moments, are never shown or written here.
The tool's text and fields are one definition (`own_memory::spec`) shared with
a Linggen session's own `memory_notes`. A host that has memory of its own
writes there and uses this only when the person asks to let every agent know.

## Browser tools

- **`browser_open`** — `op: navigate` + `url` (absolute, or back / forward);
  `op: tab_open` + `url`.
- **`browser_read`** — `op: page` (`filter`: all or interactive), `screenshot`
  (`region`), `console` (`limit`), `tabs`.
- **`browser_act`** — `op: click` (`ref` or `coordinate`, `button`, `double`),
  `type` (`text`, `ref`, `clear`), `key` (`keys`, `repeat` 1–20), `scroll`
  (`direction`, `amount`, `ref`), `wait` (`for`: load, selector or ms, and
  `value`), `tab_switch`, `tab_close`.

A Linggen session is offered the same three tools, from one definition (`engine/tools/browser_def.rs`): the names, ops, arguments and texts are the door's. The tools drive one visible Chrome tab through the extension; `page` gives refs
for `click` and `type`. A browser action can make Chrome ask the user to allow
the site; they approve or decline there. `no_bridge` means the extension is
disconnected. The X reads that were once MCP tools left the door in 2.5; the
extension's X module and `/api/bridge/*` stay for Linggen's own apps.

## Server text and usage limits

`initialize.instructions` is at or under 2,048 characters (Claude Code cuts it
there; a test holds it) and there is one text per view. The local view carries
the memory rules, the watcher rule for every `agent_run`, how to answer
`agent_permit`, the browser sentence, one line on what `app` is for, and upkeep through the dream / scan /
solve runbook with `memory_upkeep op=dream_status`. The remote view carries the
memory part only, and upkeep with the host's own model (no `agent_run`, browser
or `dream_run`).

A run that cannot go on says so; the server text tells a caller what to do on a
usage limit: read the reset time, then pick a ready model (`agent_status
op=models`) or come back after it, and never retry before it.

## Model readiness

`agent_status op=models` lists every model with `ready` true or false, and when
false, why (`not_signed_in`, `no_key`, `usage_limit` with its reset time, …)
and one line on what to do. When no model is ready the line says to add a model
in Settings → Models or sign in to Linggen Cloud; it does not push sign-up when
a local model works.

A run that ends because it cannot go on carries `cannot_run` in its `finished`
frame and in `agent_status op=session`: `reason` (`no_model`, `cloud_sign_in`,
`usage_limit`, `provider`, …), `model`, `resets_at`, `resets_in_secs`, `line`
(what to do) and `work_kept` (whether a continuation with the same `session_id`
still has what the run read). The dream's status carries the same as
`last_run_cannot_run`.

`agent_status op=session` also carries `usage`, what the session's model calls used and cost (`models.md` § What a session cost): per model `input`, `output`, `cache_read`, `cache_write` tokens and `calls`, with `cost_usd` — an estimate at list price — only for a model with a known price, a total `cost_usd` over the priced ones, and `unpriced`, the models whose tokens it leaves out. The session's total, not one run's: a continued session adds to it. A caller can tell the person what a job cost; an unpriced model has tokens and no dollars.

A run that ends at its step limit is not a failure: its status is `completed`
and `cannot_run` reads `reason: "step_limit"`, `steps` (the limit it reached),
`line` and `work_kept`. The request was fine and the job long; continue with
the same `session_id`. A coding session's limit is far higher than a chat's
(`agentic-loop.md` § Step limit).

## The old plugin

`/mcp/memory` is retired. Every request there gets HTTP 410 with
`{"ok": false, "error": "This plugin uses a retired Linggen MCP endpoint; update the Linggen plugin and restart your host.", "code": "UPGRADE_PLUGIN"}`
(an unpaired off-machine caller still gets 401 `PAIRING_REQUIRED` and a phone
403 first). Nothing is forwarded and there is no alias. A retired tool name or
a retired shape, sent to `/mcp`, runs nothing; the answer says to update the
Linggen plugin and restart the host, and names the form that replaces it. An
`agent_status` with only `session_id` is the old shape and is answered the same
way.

An old plugin's hooks swallow that answer, so the engine tells the person
instead: a call to the retired door or a retired tool name is remembered —
which host (by its client's name: Claude Code, Codex, or a host it cannot
tell), when — once per host per day (`perception/old_plugin.rs`, kept in
`activity/old-plugin.json`). `ling status` lists each host seen in the last
two days with the update commands for it, and it is a problem Yinyue raises
(`old-plugin`, perception-spec §5): said once, with those commands, since
nothing on the Mac updates a host's plugin by itself.

## Distribution

Claude Code and Codex plugins, one product name: **linggen**.

- **Claude Code plugin `linggen`**:
  - `.mcp.json` → one server, `linggen`, at the engine endpoint
    (`http://127.0.0.1:9527/mcp`).
  - Hooks: the same per-turn recall hook (`recall.sh`, CLI-based — no MCP
    round-trip in a shell hook) + session-start core load.
  - Autostart: start the daemon on `:9527` when the `ling` binary exists; when
    it doesn't, install the engine in the **background** (detached — session
    start never blocks; lock dir prevents races; `LINGGEN_NO_ENGINE_INSTALL=1`
    opts out) and disclose the install in the session context line. The engine
    is a required component of the plugin. Awareness = context line + README +
    plugin description.
  - SKILL.md: the memory protocol (ops via the `memory_*` MCP tools; CLI
    remains the fallback) plus a short browser-control section (visible tab,
    permission prompt in the browser).
  - Lives in this repo: the plugin at `plugins/linggen`, the marketplace
    files at the root (`.claude-plugin/marketplace.json`,
    `.agents/plugins/marketplace.json`; marketplace name `linggen`), one
    version with the engine. Install:
    `claude plugin marketplace add linggen/linggen --sparse .claude-plugin plugins`
    then `claude plugin install linggen@linggen`. Source:
    `github.com/linggen/linggen/tree/main/plugins/linggen`.
- **Codex plugin `linggen`** supports the same memory store and scan workflow
  (`ling mem scan <day>`). Install: `codex plugin marketplace add linggen/linggen`
  then `codex plugin add linggen@linggen`.
- **Chrome extension** (the `browser_*` tools need it): source in this repo,
  `browser-extension/`
  (`github.com/linggen/linggen/tree/main/browser-extension`).
- **Manual** (any MCP client):
  `claude mcp add --transport http linggen http://127.0.0.1:9527/mcp` or the
  equivalent in Cursor/Codex config. An approved computer adds the same URL of
  the Linggen on the owner's machine, with its token, and sees the memory tools
  only.

## Agents (`agent_run`, `agent_status`, `agent_permit`, `agent_end`)

- For code changes, create a git worktree first and pass its absolute path as
  `write_dir` to `agent_run`. Pass the host's current `permission_mode`: `plan`
  is read-only; `default`/`acceptEdits` grant edit there; `auto`/
  `bypassPermissions` grant admin there. Other paths stay read-only. Linggen
  never prompts a person for these runs; browser mutations have their own gate.
- `agent_run` accepts `work: "code"` or `"none"` for a new session: by default `write_dir` selects code, otherwise none. A continued session ignores it. A coding run gets coding guidance and runs any program where it may edit without asking (`permission-spec.md` § Coding sessions); outward commands still wait for the caller.
- A new session may be made with preset grants, so the run is not asked for
  the same thing each time: `grants: {read: […], write: […]}`. `read`
  folders are granted at read; `write` folders at `write_dir`'s level, never
  above it. A folder follows `write_dir`'s rules (an existing absolute
  folder, never `/` or home) and is never Linggen's own folder. An item the
  caller's level does not allow is refused; the run still starts with what
  survived. The reply ends with a `Preset grants:` line and `agent_status op=session`
  carries `grants`, both `{granted: {read, write}, refused: [{item,
  reason}]}`. Programs need no grant — a coding run runs them where it may
  edit (`permission-spec.md` § Coding sessions) — so `grants.tools` is an
  error saying so. With `session_id`, `grants` is an error, as `write_dir`
  is. It is described in the tool's schema only (the `instructions` text is
  near its limit).
- Give a self-contained brief: files, cause, tests to run, and “reply short”.
  Only the final reply returns; working steps remain visible in the web UI.
- A long run continues in the background. Don't poll: start a background
  watcher on `ws://127.0.0.1:9527/api/agent/wait?session_id=…`. A watch that
  closes or expires without `finished` (a host's watcher has a time cap —
  Claude Code's Monitor stops at 30 min) gets one `agent_status op=session`; if that
  still says running, the caller starts a new watch. The stream sends JSON
  frames:
  - `{"type":"permission_request","session_id","request":{id,tool,summary,command?,level,path,dir?,reason?,outward?}}`
    — once per request, including those raised before the watcher connected.
  - `{"type":"finished","session_id","status","reply","followup","denied","error","model"}`
    — last, then it closes. `status` is `completed`, `failed` or `cancelled`;
    `error` says why a failed run failed; `model` is the model that answered
    the run's last call. `agent_status op=session` carries the same `status`, `error`
    and `model`.
  - `reply` (in both) is the agent's last message in the turn that answered
    the caller's latest prompt. A turn the engine starts on its own after it —
    a background task's end — does not replace it; both `finished` and
    `agent_status op=session` give that turn's last message as `followup` (`null` if none).
    A run an engine restart interrupted goes on in the restart's continue
    turn, which still answers the prompt.
- An action beyond the grants (never the hardcoded deny floor) waits for the
  caller. Answer with `agent_permit {session_id, request_id, decision, scope?}`:
  `allow_once` runs that action; `allow_session` grants its `path` or its
  folder (`dir`, the default when grantable) at the needed level for the rest
  of that session only; `deny` refuses it and closes its project to every
  tool for the rest of that run. A request with `outward: true` is a command
  whose effect leaves the machine (`permission-spec.md` § Outward commands):
  it is asked whatever the run's grants and takes `allow_once` or `deny`
  only. No answer in 10 min counts as deny; `agent_end op=stop`
  ends a waiting run at once.
  `agent_run` returns early when a request is raised; `agent_status op=session` lists
  open requests in `pending`. Refusals land in `denied` with their reason.
- A session made by `agent_run` does not fall back to another model when its
  model fails (`model_fallback`, default `false`; pass `true` to allow it):
  the run ends `failed`, `error` naming the model and the provider's reason, and
  the caller decides. The web UI's model switch 「出错时自动换模型」 shows and sets
  the same per-session value.
- Follow up with `agent_run(session_id=…)` to retain context; omit `write_dir`,
  `permission_mode`, and `model`. The session keeps its first grants, its
  session grants, and its toolset; continuing never widens them. `agent_status op=models` lists models for new sessions;
  `agent_end op=stop` cancels a running turn and its subagents.
- Review the diff before merging. When done, call
  `agent_end op=close` with `session_id` and `remove_worktree=true`; removal requires a
  clean linked worktree whose HEAD is contained in a remote branch.
- The MCP `instructions` string stays at or under 2048 characters: Claude
  Code cuts it there (a test holds it).

## Later

- Group toggles in daemon config for hosts that want a narrower surface.

## History — how the door got here

Kept so the decisions are not lost. Everything in this section is retired.

- **Before 2.3** memory had its own daemon on `127.0.0.1:9528`, and from
  2026-07-30 the engine's `/mcp` (24 tools: `browser_*`, `x_*`, `agent_*`,
  `memory_dream_*`) and memory's own server (17 tools) were two MCP servers.
  The plugin shipped both entries. The reason for two was that two servers
  offering the same memory tools would be two `memory_search` tools with
  identical schemas, which the model picks between arbitrarily.
- **2.3** moved memory into the engine: one process, one port. Memory's server
  became `/mcp/memory` on 9527, and 9528 stopped answering.
- **2.5** (2026-11-18, the owner's call) merged the two servers into this one
  door and cut 41 tools to twelve. The reason: memory needs the engine (dream,
  scan, review), and one install should show the other tools, such as
  `agent_run`, beside it.
- **2.7** (2026-11-21, the owner's call) added a thirteenth tool, `app`: an
  outside agent reaches the person's apps the way a session does, never the
  file and shell tools.

### Retired in 2.5 — what replaced each

Calling a retired name runs nothing; the answer names the new form.

| Retired | Replaced by |
|:--------|:------------|
| `memory_list` | `memory_search` `op=list` |
| `memory_get` | `memory_search` `op=get` |
| `memory_session_start` | `memory_upkeep` `op=session_start` |
| `memory_harvest_day`, `memory_days`, `memory_remember_day`, `memory_dream_lease`, `memory_role`, `memory_sweep`, `memory_issues`, `memory_issue_add`, `memory_issue_resolve`, `memory_chains`, `memory_dream_status`, `memory_dream_run` | `memory_upkeep`, `op` = the name after `memory_` |
| `agent_stop` | `agent_end` `op=stop` |
| `agent_session_close` | `agent_end` `op=close` |
| `agent_models` | `agent_status` `op=models` |
| `browser_navigate` | `browser_open` `op=navigate` |
| `browser_tabs` | `browser_open` `op=tab_open`, `browser_read` `op=tabs`, `browser_act` `op=tab_switch` or `tab_close` |
| `browser_read_page`, `browser_screenshot`, `browser_read_console` | `browser_read` `op=page`, `screenshot`, `console` |
| `browser_click`, `browser_type`, `browser_key`, `browser_scroll`, `browser_wait` | `browser_act` `op=click`, `type`, `key`, `scroll`, `wait` |
| `x_search`, `x_targets`, `x_following`, `x_whotofollow`, `x_own`, `x_mentions` | none — they left the door |

Kept names: `memory_search`, `memory_add`, `memory_update`, `memory_delete`,
`agent_run`, `agent_status`, `agent_permit`. The `mcp__plugin_linggen_memory__*`
tool namespace is gone with the second server.

## Phasing

1. Memory on `/mcp` (2026-07), taken out for its own server (2026-07-30),
   served by the engine from 2.3. History.
2. **2.5: one door, twelve tools.** Done.
3. **2.7: the thirteenth tool, `app`.** Done.
