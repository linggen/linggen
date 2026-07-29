---
type: spec
reader: Coding agent and users
guide: |
  Product specification — describe what the system should do and why.
  Keep it brief. Aim to guide design and implementation, not document code.
status: designed 2026-07-29, not built. Phase 1 has independent value; phase 2 depends on it.
---

# Ling as an MCP client

Linggen can be *driven by* MCP and cannot *consume* it. `src/server/mcp.rs`
and `src/server/mcp_agent.rs` are both servers; there is no outbound
`tools/list` anywhere in the tree.

Every peer — Claude Code, Codex, Cursor, Cline, Zed — connects to arbitrary
MCP servers. Linggen connects to none. For a product whose premise is an OS
for agents, that is an OS that cannot load third-party drivers. This is the
gap to close.

The agent `ling` is a model in a loop, exactly like those peers. Its tools
belong on a model-facing protocol. That is a different question from how the
*engine process* talks to its own components — see "What stays REST".

## Phase 1 — the generic client

A user adds an MCP server the way they would in any other agent, and its
tools appear in the session.

- **Config** — `[[mcp_servers]]` in `linggen.runtime.toml`, mirroring Claude
  Code's `.mcp.json` and Codex's `[mcp_servers]` so a user can copy an entry
  across. Name, transport, command/args/env or url/headers.
- **Transports** — stdio (what most servers ship) and streamable HTTP (what
  ours ships). Both, or the client is only half useful.
- **Protocol** — `initialize`, `tools/list`, `tools/call`, and the
  notifications. `initialize.instructions` is not decoration: it is how a
  server ships its own doctrine, and phase 2 depends on honouring it.
- **Tool surface** — discovered tools join the session's tool list,
  name-prefixed by server so two servers can both offer `search`.
- **Permission** — user-added tools land **inside** `permission-spec.md`, not
  beside it. An external server that writes files or calls an API is exactly
  what the permission model is for. They are never Chat-tier.
- **Failure** — a server that is slow, dead, or never starts must not wedge a
  turn. Discovery is best-effort per session; a missing server means missing
  tools and a line saying so, never a hang.

## Phase 2 — ling-mem as the built-in server

Memory stops being a hand-built special case (`engine/tools/memory_tool.rs`,
`Memory_query` / `Memory_write`) and becomes the MCP server that ships
enabled. One mechanism for every tool.

**Why it is worth doing beyond tidiness:** the memory doctrine is currently
hand-copied across three surfaces that drift — ling-mem's `mcp.rs`
`INSTRUCTIONS`, the engine's `prompts/system-prompt.toml [memory_protocol]`,
and the skill's `SKILL.md`. A client that honours `initialize.instructions`
receives the doctrine from the server, and the engine stops carrying its own
copy. Three surfaces become two.

It also makes memory replaceable — point at a different memory server — which
is what "platform" has to mean.

### What stays REST

Auto-recall injects recalled rows into the prompt **before the model runs**.
There is no model, so there is no tool call, so it cannot be MCP. That path
stays `call_memory_http` against `ling_mem_url`, and `ling_mem_url` does not
go away.

Usefully, per-skill scoping (`memory-context`, which CFO relies on) lives on
exactly that path — `server/chat/runtime.rs:389`, `server/chat/handler.rs:519`
— not on the tool call. So scoping is not part of the migration.

The same split already exists in the Claude Code plugin, and is worth copying
rather than fighting: `hooks/recall.sh` **reads** over the `ling-mem` CLI,
then injects text telling the model to **write** through the MCP verbs. A hook
has no model; the model has no shell. Each uses what it can reach.

### The permission question

`Memory_query` / `Memory_write` are **Chat-tier — ungated, no prompts**
(`engine/permission/model.rs:653`). A user-added MCP server must not be. So
the permission model needs to distinguish the built-in server from added
ones, or every memory write starts prompting.

Decide this deliberately. It is the one part of phase 2 that is a design
choice rather than a mechanical change.

### Migration

Mechanical, but it touches a lot: `Memory_query` / `Memory_write` become
`memory_search` / `memory_add` / … across `agents/*.md` tool lists,
`prompts/system-prompt.toml`, `SKILL.md`, and CFO's `allowed-tools`.

## What this makes unnecessary

**A remote-memory gateway on the engine.** Once ling is an MCP client, a
second machine's ling connects to the first machine's `/mcp` exactly the way
Claude Code does — same URL, same `x-linggen-device` gate, same `memory_*`
tools. No proxy route, no exposing `ling-mem` beyond loopback, no second auth
system. An `/api/memory/*` passthrough would be dead code the day this lands;
do not build it.

Its engine-internal auto-recall is a separate question, answered by
`ling_mem_url` pointing at a reachable daemon — see `network-spec.md`.

## Sequencing

Phase 1 pays off the day it lands: GitHub, Playwright, Sentry, Figma and
every other MCP server become reachable from Linggen. Phase 2 is a follow-on.
Bundled together, a memory migration blocks a capability that stands on its
own.
