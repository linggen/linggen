---
type: spec
reader: Coding agent and users
guide: |
  Product specification — describe what the system should do and why.
  Keep it brief. Aim to guide design and implementation, not document code.
status: browser_* + x_* + memory_* live; linggen plugin + ClawHub skill next
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
| `agent_*` | Linggen agents (delegate a task) | later |

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
- The server `instructions` carry the memory protocol (three tiers, voice law,
  `source_session`, `replace_ids`/`user_directed` guard) — same text the
  ling-mem MCP ships today.
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
  - Autostart: ensure the engine is installed (`install.sh`) and the daemon is
    up on `:9898`; the engine auto-installs ling-mem on first memory use.
  - SKILL.md: the memory protocol (ops via `memory_*` MCP tools — this
    supersedes the old plugin's CLI-only rule; CLI remains the fallback) plus
    a short browser-control section (visible tab, permission prompt in the
    browser).
  - Lives in the `linggen-memory` repo beside the old plugin, same
    marketplace; the repo can be renamed later without breaking installs.
- **ClawHub skill `linggen`** (OpenClaw): same shape — SKILL.md + MCP config
  pointing at `:9898/mcp`; supersedes the `ling-mem` ClawHub listing.
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
- **ClawHub `ling-mem` skill** — listing updated to point at `linggen`.
- **ling-mem's own MCP server** — code stays (harmless) but is no longer
  promoted anywhere; docs and install pages route everyone to the linggen
  endpoint. The ling-mem binary/daemon itself is unchanged — it is the memory
  engine behind the proxy.
- Site installers: `install-shared-memory.sh` retires; `install.sh` is the
  base for every channel.

## Later

- `agent_*` — `agent_run(prompt, agent?)`: delegate a task to a local Linggen
  agent (skills + memory + local models) and return its result over MCP. The
  group no other vendor can copy cheaply.
- Group toggles in daemon config for hosts that want a narrower surface.

## Phasing

1. **memory_* on `/mcp`** — proxy + instructions + autostart path (engine). Done.
2. **`linggen` plugin** — new plugin, shared-memory retirement notes.
3. **ClawHub `linggen` skill** + listing updates.
4. **Site/docs** — install pages route to the one endpoint.
5. Later: `agent_*`.
