---
type: spec
reader: Coding agent and users
guide: |
  Product specification — describe what the system should do and why.
  Keep it brief. Aim to guide design and implementation, not document code.
status: all shipped — /mcp live (22 tools incl agent_run), linggen plugin + ClawHub skill published, site/docs routed
---

# Linggen MCP — the capability front door

One MCP server, on the daemon, for **every** outside agent (Claude Code, Codex,
Cursor, OpenClaw…): `http://127.0.0.1:9527/mcp`, streamable HTTP, stateless
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

**Cut, not a long window** (2026-07-30, Liang's call). The group went out in
the same release that gave the plugin its second MCP entry. The plugin was the
only channel that ever wired this door for memory, so it migrates its users
atomically; leaving both would have put two `memory_search` tools with
identical schemas in front of one model, which is the duplication this whole
arc exists to remove — self-inflicted. Anyone who wired `:9527` by hand gets
`unknown tool` and this page.

## memory_* group — REMOVED

Gone: `memory_search`, `memory_add`, `memory_get`, `memory_update`,
`memory_delete`, `memory_list`, `memory_issues`, `memory_issue_resolve`. They
are ling-mem's, served on `127.0.0.1:9528/mcp`. Add that server.

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
  first-use autostart (install missing ling-mem, start `:9528`) fires for MCP
  callers too. ling-mem unreachable after autostart → friendly install-guidance
  tool error, mirroring `no_bridge`.

## Distribution

Three channels, one product name: **linggen**.

- **Claude Code plugin `linggen`** (replaces `shared-memory`):
  - `.mcp.json` → the daemon endpoint (`http://127.0.0.1:9527/mcp`).
  - Hooks: the same per-turn recall hook (`recall.sh`, CLI-based — no MCP
    round-trip in a shell hook) + session-start core load.
  - Autostart: start the daemon on `:9527` when the `ling` binary exists; when
    it doesn't, install the engine in the **background** (detached — session
    start never blocks; lock dir prevents races; `LINGGEN_NO_ENGINE_INSTALL=1`
    opts out) and disclose the install in the session context line. Both
    binaries are required components of the plugin — decided 2026-07-10,
    reversing the earlier hint-only rule. Awareness = context line + README +
    plugin description; hook-less channels (ClawHub, skills.sh) get the same
    via the SKILL.md first-use gate (agent announces, then installs).
    ling-mem still bootstraps itself (the recall hook needs its CLI).
  - SKILL.md: the memory protocol (ops via `memory_*` MCP tools — this
    supersedes the old plugin's CLI-only rule; CLI remains the fallback) plus
    a short browser-control section (visible tab, permission prompt in the
    browser).
  - Lives in the `linggen-memory` repo beside the old plugin, same
    marketplace; the repo can be renamed later without breaking installs.
- **ClawHub skill `linggen`** (OpenClaw): the `ling-mem` listing was renamed
  in place (old slug redirects, history kept) and republished as `linggen`
  2.0.0 — same SKILL.md as the plugin.
- **Manual** (any MCP client):
  `claude mcp add --transport http linggen http://127.0.0.1:9527/mcp` or the
  equivalent in Cursor/Codex config.

## Retirement map

No real user base yet (2026-07-10), so retirement is a clean cut, not a
deprecation window:

- **`shared-memory` CC plugin — removed outright** from the marketplace; the
  plugin directory is replaced by `linggen`. The one live install (the dev
  machine) migrates as part of Phase 2 verification — never run both, two
  plugins means a doubled recall hook.
- **ClawHub `ling-mem` skill** — renamed to `linggen` (slug redirect). Done.
- **ling-mem's own MCP server** — ~~code stays (harmless) but is no longer
  promoted anywhere~~. **Reversed 2026-07-29**: it is the promoted memory
  endpoint. Everything of Linggen's is on it — the engine's agents via its MCP
  client, a second machine on the LAN directly — and the plugin pointing
  outside agents there is what closes the window above.
- Site installers: `install-shared-memory.sh` is a guidance stub pointing at
  the plugin channels; `install.sh` is the base for every channel. Done.

## agent_* group

`agent_run(prompt, agent?)` — delegate a task to a **local Linggen agent** (this
machine's skills, memory, and configured models) and return its final reply.
The capability no generic tool server can copy: it runs the user's own agent.

- One-shot, headless: a fresh visible session, the agent loop runs to
  completion, the last assistant message is returned. Unknown `agent` returns
  the available list.
- **Safe by default.** The delegate is non-interactive (a headless MCP caller
  can't answer a Linggen-side prompt, so a permission-needed action silently
  denies and the agent continues) and runs a **read/memory/browser toolset
  only** — no Bash, Write, Edit, or Task, so the read-only boundary can't be
  worked around via a shell redirect. Browser mutations still pass the
  extension's own gate. Widening to a write mode is a future opt-in.

## Later

- A `write` mode on `agent_run` (opt-in Bash/Write/Edit for trusted callers).
- Group toggles in daemon config for hosts that want a narrower surface.

## Phasing

1. **memory_* on `/mcp`** — proxy + instructions + autostart path (engine). Done.
2. **`linggen` plugin** — new plugin, shared-memory retirement notes. Done.
3. **ClawHub `linggen` skill** + listing updates. Done.
4. **Site/docs** — install pages route to the one endpoint. Done.
5. Later: `agent_*`.
