---
type: spec
reader: Coding agent and users
guide: |
  Product specification — describe what the system should do and why.
  Keep it brief. Aim to guide design and implementation, not document code.
status: all shipped — /mcp live (24 tools incl agent_run, agent_permit), linggen plugin published, site/docs routed
---

# Linggen MCP — the capability front door

One MCP server, on the daemon, for **every** outside agent (Claude Code, Codex,
Cursor and others): `http://127.0.0.1:9527/mcp`, streamable HTTP, stateless
JSON-RPC (`src/server/mcp.rs`). Tools are grouped by prefix; each group fronts
a Linggen component the daemon already talks to. One config line installs the
whole platform's capabilities; every new group makes the same install more
valuable.

| Group | Fronts | Status |
|:------|:-------|:-------|
| `browser_*` | linggen-browser extension (control module) | live |
| `x_*` | linggen-browser extension (x session reads) | live |
| ~~`memory_*`~~ | — | **REMOVED 2026-07-30** — served by ling-mem's own `/mcp`, see below |
| `memory_dream_*` | the engine's own mission executor | live, and staying — ling-mem cannot serve these |
| `agent_*` | Linggen agents (delegate a task) | live |

~~Decided (2026-07-10): one MCP for all users — including memory-only users.~~
**REVERSED 2026-07-29** — see `mcp-client-spec.md`.

The original reasoning was that two servers offering the same memory tools
would confuse migrating users. That holds, and it is now the argument for the
other answer: once `ling` becomes an MCP *client* and the plugin ships both
servers, a proxied `memory_search` and a direct one are two tools with
identical schemas, and the model picks between them arbitrarily. The way to
have one memory tool is to serve it in one place.

So **`memory_*` leaves this front door.** Memory is served by ling-mem, whose
`/mcp` has existed since 2026-05-27 and is part of the frozen 1.x contract;
this door keeps `browser_*`, `x_*` and `agent_*`. A host adds the servers
whose capabilities it wants.

The old decision's second premise also weakened: a ling-mem-only install
can't run the dream missions, which is still true, but that is an argument
for shipping both binaries — which the plugin already does — not for hiding
one behind the other.

**Cut, not a long window** (2026-07-30, Hanli's call). The group went out in
the same release that gave the plugin its second MCP entry. The plugin was the
only channel that ever wired this door for memory, so it migrates its users
atomically; leaving both would have put two `memory_search` tools with
identical schemas in front of one model, which is the duplication this whole
arc exists to remove — self-inflicted. Anyone who wired `:9527` by hand gets
`unknown tool` and this page.

## memory_* group — REMOVED

Gone: `memory_search`, `memory_add`, `memory_get`, `memory_update`,
`memory_delete`, `memory_list`, `memory_issues`, `memory_issue_resolve`. They
are memory's, served on `/mcp/memory` on the engine's port (before 2.3:
ling-mem's `127.0.0.1:9528/mcp`). Add that server.

**`memory_dream_status` and `memory_dream_run` stay** — they wear the
`memory_` prefix but are *engine* capabilities: the first composes ling-mem's
days rollup with the engine's in-flight run state, the second drives the
mission executor. ling-mem can serve neither.
- **Dream + review-queue tools** (2026-07-17): `memory_dream_status` (daemon
  days rollup + open review items + in-flight flag + last run outcome, with
  `last_run_error` pulled from a failed run's session tail so the host can
  show the user why), `memory_dream_run` (triggers the dream mission through
  `trigger_mission_core` — the same guarded path as the HTTP trigger; quiet
  variant only, since MCP callers can't receive AskUser), `memory_issues` and
  `memory_issue_resolve` (proxy the daemon's review-queue sidecar — facts and
  bookkeeping; the calling agent is the solver). Hosts are steered to run the
  dream with their own model (`/linggen:dream`) and use `memory_dream_run`
  only to offload to the engine's executor.
- The server `instructions` carry the memory protocol (three tiers, voice law,
  `source_session`, `replace_ids`/`user_directed` guard, the status-supersede
  rule, and when to offer dream/solve) — same text the ling-mem MCP ships
  today.
- Proxy through the engine's existing ling-mem HTTP client path so the
  first-use autostart (install missing ling-mem, start its port) fires for MCP
  callers too. ling-mem unreachable after autostart → friendly install-guidance
  tool error, mirroring `no_bridge`.

## Distribution

Claude Code and Codex plugins, one product name: **linggen**.

- **Claude Code plugin `linggen`**:
  - `.mcp.json` → the daemon endpoint (`http://127.0.0.1:9527/mcp`).
  - Hooks: the same per-turn recall hook (`recall.sh`, CLI-based — no MCP
    round-trip in a shell hook) + session-start core load.
  - Autostart: start the daemon on `:9527` when the `ling` binary exists; when
    it doesn't, install the engine in the **background** (detached — session
    start never blocks; lock dir prevents races; `LINGGEN_NO_ENGINE_INSTALL=1`
    opts out) and disclose the install in the session context line. Both
    binaries are required components of the plugin — decided 2026-07-10,
    reversing the earlier hint-only rule. Awareness = context line + README +
    plugin description.
    ling-mem still bootstraps itself (the recall hook needs its CLI).
  - SKILL.md: the memory protocol (ops via `memory_*` MCP tools — this
    supersedes the old plugin's CLI-only rule; CLI remains the fallback) plus
    a short browser-control section (visible tab, permission prompt in the
    browser).
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
- **Chrome extension** (the `browser_*` and `x_*` groups need it): source in
  this repo, `browser-extension/`
  (`github.com/linggen/linggen/tree/main/browser-extension`).
- **Manual** (any MCP client):
  `claude mcp add --transport http linggen http://127.0.0.1:9527/mcp` or the
  equivalent in Cursor/Codex config. Any MCP-capable agent can add
  `http://127.0.0.1:9527/mcp/memory` as an MCP server (tools only).

## Retirement map

No real user base yet (2026-07-10), so retirement is a clean cut, not a
deprecation window:

- Memory rows, days and the review queue are in Settings → Memory in Linggen's
  UI; run `/linggen:scan` from the Claude Code / Codex plugin.
- **ling-mem's own MCP server** — ~~code stays (harmless) but is no longer
  promoted anywhere~~. **Reversed 2026-07-29**: it is the promoted memory
  endpoint. Everything of Linggen's is on it — the engine's agents via its MCP
  client, a second machine on the LAN directly — and the plugin pointing
  outside agents there is what closes the window above.
- Site installers: the retired memory installer points at the plugin channels;
  `install.sh` is the base for every channel. Done.

## agent_* group

- For code changes, create a git worktree first and pass its absolute path as
  `write_dir` to `agent_run`. Pass the host's current `permission_mode`: `plan`
  is read-only; `default`/`acceptEdits` grant edit there; `auto`/
  `bypassPermissions` grant admin there. Other paths stay read-only. Linggen
  never prompts a person for these runs; browser mutations have their own gate.
- `agent_run` accepts `work: "code"` or `"none"` for a new session: by default `write_dir` selects code, otherwise none. A continued session ignores it.
- A new session may be made with preset grants, so the run is not asked for
  the same thing each time: `grants: {tools: […], read: […], write: […]}`.
  `tools` are build tools from the owner's list (`permission-spec.md`
  § Granted tools) the run may use where it may edit; they need a `write_dir`
  at edit or admin. `read` folders are granted at read; `write` folders at
  `write_dir`'s level, never above it. A folder follows `write_dir`'s rules
  (an existing absolute folder, never `/` or home) and is never Linggen's own
  folder. An item the owner's list or the caller's level does not allow is
  refused; the run still starts with what survived. The reply ends with a
  `Preset grants:` line and `agent_status` carries `grants`, both
  `{granted: {tools, read, write}, refused: [{item, reason}]}`. With
  `session_id`, `grants` is an error, as `write_dir` is. It is described in
  the tool's schema only (the `instructions` text is near its limit).
- Give a self-contained brief: files, cause, tests to run, and “reply short”.
  Only the final reply returns; working steps remain visible in the web UI.
- A long run continues in the background. Don't poll: start a background
  watcher on `ws://127.0.0.1:9527/api/agent/wait?session_id=…`. A watch that
  closes or expires without `finished` (a host's watcher has a time cap —
  Claude Code's Monitor stops at 30 min) gets one `agent_status`; if that
  still says running, the caller starts a new watch. The stream sends JSON
  frames:
  - `{"type":"permission_request","session_id","request":{id,tool,summary,command?,level,path,dir?,reason?,outward?}}`
    — once per request, including those raised before the watcher connected.
  - `{"type":"finished","session_id","status","reply","denied","error","model"}`
    — last, then it closes. `status` is `completed`, `failed` or `cancelled`;
    `error` says why a failed run failed; `model` is the model that answered
    the run's last call. `agent_status` carries the same `status`, `error`
    and `model`.
  - `reply` (in both) is the agent's last message in the turn that answered
    the caller's latest prompt. A turn the engine starts on its own after it —
    a background task's end — does not replace it; `agent_status` gives that
    turn's last message as `followup`. A run an engine restart interrupted
    goes on in the restart's continue turn, which still answers the prompt.
- An action beyond the grants (never the hardcoded deny floor) waits for the
  caller. Answer with `agent_permit {session_id, request_id, decision, scope?}`:
  `allow_once` runs that action; `allow_session` grants its `path` or its
  folder (`dir`, the default when grantable) at the needed level for the rest
  of that session only; `deny` refuses it and closes its project to every
  tool for the rest of that run. A request with `outward: true` is a command
  whose effect leaves the machine (`permission-spec.md` § Outward commands):
  it is asked whatever the run's grants and takes `allow_once` or `deny`
  only. No answer in 10 min counts as deny; `agent_stop`
  ends a waiting run at once.
  `agent_run` returns early when a request is raised; `agent_status` lists
  open requests in `pending`. Refusals land in `denied` with their reason.
- A session made by `agent_run` does not fall back to another model when its
  model fails (`model_fallback`, default `false`; pass `true` to allow it):
  the run ends `failed`, `error` naming the model and the provider's reason, and
  the caller decides. The web UI's model switch 「出错时自动换模型」 shows and sets
  the same per-session value.
- Follow up with `agent_run(session_id=…)` to retain context; omit `write_dir`,
  `permission_mode`, and `model`. The session keeps its first grants, its
  session grants, and its toolset; continuing never widens them. `agent_models` lists models for new sessions;
  `agent_stop` cancels a running turn and its subagents.
- Review the diff before merging. When done, call
  `agent_session_close(session_id, remove_worktree=true)`; removal requires a
  clean linked worktree whose HEAD is contained in a remote branch.
- The MCP `instructions` string stays at or under 2048 characters: Claude
  Code cuts it there (a test holds it).

## Later

- Group toggles in daemon config for hosts that want a narrower surface.

## Phasing

1. **memory_* on `/mcp`** — proxy + instructions + autostart path (engine). Done.
2. **`linggen` plugin** — new plugin. Done.
3. **Site/docs** — install pages route to the one endpoint. Done.
4. Later: `agent_*`.
