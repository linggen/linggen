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
Cursor, OpenClaw…): `http://127.0.0.1:9898/mcp`, streamable HTTP, stateless
JSON-RPC (`src/server/mcp.rs`). Tools are grouped by prefix; each group fronts
a Linggen component the daemon already talks to. One config line installs the
whole platform's capabilities; every new group makes the same install more
valuable.

| Group | Fronts | Status |
|:------|:-------|:-------|
| `browser_*` | linggen-browser extension (control module) | live |
| `x_*` | linggen-browser extension (x session reads) | live |
| `memory_*` | ling-mem daemon (`:9888`) | live |
| `agent_*` | Linggen agents (delegate a task) | live |

Decided (2026-07-10): **one MCP for all users — including memory-only users.**
Two servers offering the same memory tools would confuse anyone migrating, and
a ling-mem-only install can't run missions (dream, nightly condense) — the
engine brings those. So the engine is the base install for every channel;
ling-mem remains the memory component the engine manages and proxies, not a
separately-promoted MCP.

## memory_* group

Thin proxy to the ling-mem daemon — no code moves between repos.

- Tools: `memory_search`, `memory_add`, `memory_get`, `memory_update`,
  `memory_delete`, `memory_list`. Names and schemas mirror ling-mem's MCP so
  migrating users keep muscle memory. Dream-pipeline verbs (`harvest_day`,
  `remember_day`, `sweep`, `chains`, `days`) stay engine-internal — missions
  run them; third-party agents don't.
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
  first-use autostart (install missing ling-mem, start `:9888`) fires for MCP
  callers too. ling-mem unreachable after autostart → friendly install-guidance
  tool error, mirroring `no_bridge`.

## Distribution

Three channels, one product name: **linggen**.

- **Claude Code plugin `linggen`** (replaces `shared-memory`):
  - `.mcp.json` → the daemon endpoint (`http://127.0.0.1:9898/mcp`).
  - Hooks: the same per-turn recall hook (`recall.sh`, CLI-based — no MCP
    round-trip in a shell hook) + session-start core load.
  - Autostart: start the daemon on `:9898` when the `ling` binary exists; when
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
  `claude mcp add --transport http linggen http://127.0.0.1:9898/mcp` or the
  equivalent in Cursor/Codex config.

## Retirement map

No real user base yet (2026-07-10), so retirement is a clean cut, not a
deprecation window:

- **`shared-memory` CC plugin — removed outright** from the marketplace; the
  plugin directory is replaced by `linggen`. The one live install (the dev
  machine) migrates as part of Phase 2 verification — never run both, two
  plugins means a doubled recall hook.
- **ClawHub `ling-mem` skill** — renamed to `linggen` (slug redirect). Done.
- **ling-mem's own MCP server** — code stays (harmless) but is no longer
  promoted anywhere; docs and install pages route everyone to the linggen
  endpoint. The ling-mem binary/daemon itself is unchanged — it is the memory
  engine behind the proxy.
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
