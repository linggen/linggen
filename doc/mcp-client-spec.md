---
type: spec
reader: Coding agent and users
guide: |
  Product specification — describe what the system should do and why.
  Keep it brief. Aim to guide design and implementation, not document code.
status: PHASE 1 BUILT 2026-07-29 (client, tool surface, project scope, McpTab). Phase 2 not started.
---

# Ling as an MCP client

Linggen can be *driven by* MCP and cannot *consume* it. `src/server/mcp.rs`
and `src/server/mcp_agent.rs` are both servers; there is no outbound
`tools/list` anywhere in the tree.

Every peer — Claude Code, Codex, Cursor, Cline, Zed — connects to arbitrary
MCP servers. Linggen connects to none. For a product whose premise is an OS
for agents, that is an OS that cannot load third-party drivers. This is the
gap to close.

## A model is needed to CHOOSE a tool, not to CALL one

This is the load-bearing fact for everything below, and it is easy to get
wrong (this spec's first draft did). MCP is JSON-RPC. The client is a
program. Verified against the running daemon with nothing but curl — no
agent, no model, no `initialize` handshake, because the server is stateless:

```
POST 127.0.0.1:9528/mcp
{"jsonrpc":"2.0","id":1,"method":"tools/call",
 "params":{"name":"memory_search","arguments":{"query":"…","limit":2}}}
→ 2 rows
```

So anything in the engine can call an MCP tool, including code paths that run
before the model. That is what makes auto-recall over MCP possible, and it
removes the last reason for a separate memory address.

## Phase 1 — the generic client — BUILT

`src/mcp_client/` (config, transport, client, registry), `[mcp_servers]` in
`linggen.runtime.toml`, `GET /api/mcp`, and Settings → MCP. Commits `7c07dec`,
`2b851f2`, `bfcf576`, `1738049`.

A user adds an MCP server the way they would in any other agent, and its
tools appear in the session.

- **Config** — `[mcp_servers]` in `linggen.runtime.toml`, with the same field
  names as the de-facto `mcpServers` JSON shape (`command`/`args`/`env`, or
  `type`/`url`/`headers`) so an entry transliterates from Claude Code's
  `.mcp.json` without thinking. MCP standardises the protocol, not where a
  host stores its server list — CC uses `.mcp.json` + `~/.claude.json`, Codex
  uses `[mcp_servers]`, Cursor uses `.cursor/mcp.json`. The shape is what
  travels.
- **One scope.** Every server is the user's own, in `[mcp_servers]`. A repo's
  `.mcp.json` is **not** read. Project scope shipped in phase 1 and was
  removed 2026-07-29: a stdio entry is `command` + `args`, so honouring a
  file inside a cloned repo means launching whatever it names, and nothing
  gated it. CC offers the same feature only behind an approval prompt, and
  since v2.1.196 refuses to let a repo approve itself. The engine had nowhere
  sound to put that prompt — it resolved *one* workspace root at daemon boot
  and applied it to every session, including sessions working elsewhere. Not
  global, not per-project: the boot cwd, frozen.
- **Transports** — stdio (what most servers ship) and streamable HTTP (what
  ours ships). Not WebRTC: the *client* picks the transport and we do not own
  CC's, so a WebRTC dialect would be speakable by exactly one client, which
  defeats the purpose of being a server.
- **Protocol** — `initialize`, `tools/list`, `tools/call`, notifications.
  `initialize.instructions` is not decoration; phase 2 depends on honouring
  it.
- **Tool surface** — discovered tools join the session's tool list,
  name-prefixed by server so two servers can both offer `search`.
- **Failure** — a server that is slow, dead, or never starts must not wedge a
  turn. Discovery is best-effort per session; a missing server means missing
  tools and a line saying so, never a hang.

### The permission question — DECIDED 2026-07-29: follow Claude Code

`Memory_query` / `Memory_write` are **Chat-tier — ungated, no prompts**
(`engine/permission/model.rs`). A user-added MCP server must not be: its
tools can write files and call APIs, which is precisely what
`permission-spec.md` exists for.

First, the premise to discard: **MCP has no permission model to inherit.**
The protocol defines transport, discovery, and *advisory* tool annotations,
and says those annotations are untrusted hints from the server. "Follow the
MCP standard" is not an answer.

So we follow CC's, **for MCP tools only** — built-in tools keep the
chat/read/edit/admin ladder and `path_modes[]`. That is two vocabularies in
one engine, accepted deliberately: the alternative rewrites a permission
model that was just simplified.

- **Rules are `deny` → `ask` → `allow`**, matched on `mcp__<server>` (whole
  server), `mcp__<server>__*`, or `mcp__<server>__<tool>`. An allow glob must
  be anchored after a literal server segment — an unanchored one approves
  nothing. No match falls through to the session's permission mode.
- **A server may escalate, never de-escalate.** CC honours
  `_meta["anthropic/requiresUserInteraction"]` to force a prompt on every
  call that no allow rule can skip, and has no inverse: a server cannot
  declare itself safe. So `readOnlyHint` must never *widen* access. Trust
  flows one way, from the user's own config.
- **No Chat tier for MCP.** The built-in memory server is un-gated by a
  shipped-default allow rule the user can see and revoke, not by a hardcoded
  tier — which also keeps `memory_*` from being privileged wherever it comes
  from.
- **Auto-recall is not gated at all.** It is engine code acting for the user
  before the model runs, not the agent choosing a tool, and permission gates
  the agent.

### The Settings tab

Config, server struct and web form are three views of one schema, the same
way `[[models]]` and `[[agents]]` already are (`POST /api/config` →
`update_config_api`). An McpTab follows that pattern. Three things it must
show, or it violates the show-everything rule:

- **Every server it connects to**, and only those. With project scope gone
  there is one list and it is the user's, so the tab can no longer show a
  server the user did not add — the hidden-state risk this bullet used to
  guard against is now structural.
- **The permission tier per server.** A tab that adds a server without
  showing what it may do is a phantom: real capability, invisible.
- **Live connection state** — connected / failed / N tools discovered, read
  from the client. Not a green dot meaning "configured".

## Phase 2 — ling-mem as the built-in server

Memory stops being a hand-built special case (`engine/tools/memory_tool.rs`,
`Memory_query` / `Memory_write`) and becomes the MCP server that ships
enabled — one entry in `[mcp_servers]`, one mechanism for every tool.

**Why it is worth doing beyond tidiness:** the memory doctrine is currently
hand-copied across three surfaces that drift — ling-mem's `mcp.rs`
`INSTRUCTIONS`, the engine's `prompts/system-prompt.toml [memory_protocol]`,
and the skill's `SKILL.md`. A client that honours `initialize.instructions`
receives the doctrine from the server, and the engine stops carrying its own
copy. Three surfaces become two.

It also makes memory replaceable — point at a different memory server —
which is what "platform" has to mean.

### Auto-recall

Auto-recall injects rows into the prompt before the model runs. It is not a
tool *choice*, but it is a tool *call*, so it goes through the engine's own
client like everything else:

```
1. engine takes the user's prompt
2. its MCP client → tools/call memory_search
     { query: <prompt>, contexts: [...], limit: K }
3. → the memory server in [mcp_servers]
4. filter by min_score, inject top-K
5. model runs
```

Local and remote are the same code with a different URL. Two details:

- **`contexts` already exists** in ling-mem's `memory_search` schema, so
  per-skill scoping (`memory-context`, which CFO relies on) survives intact.
- **`min_score` does not.** Either add it to the schema or filter client-side
  on the `hybrid_score` rows already carry.

**Fail soft is a hard requirement.** If the memory server is unreachable,
recall injects nothing, says so once, and the turn proceeds. Never a hang,
never a failed turn — the engine has `--idle-shutdown-secs 300`, so an absent
daemon is an ordinary condition, not an exception.

### Recall over MCP — including from a hook

A hook cannot *be* a model's tool call. It can perfectly well *make* an MCP
call: `tools/call` is an HTTP POST with a JSON-RPC body, which `curl` does.
An earlier draft of this spec claimed `recall.sh` "can never be MCP"; that was
wrong, and it mattered.

```bash
curl -s "$LINGGEN_MEMORY_URL" -H 'content-type: application/json' \
     -H "x-linggen-device: $TOKEN" \
     -d '{"jsonrpc":"2.0","id":1,"method":"tools/call",
          "params":{"name":"memory_search","arguments":{"query":"…"}}}' \
  | jq -r '.result.content[0].text'
```

`recall.sh` already depends on `jq`, and rows carry `hybrid_score`, `score`
and `contexts`, so the missing `min_score` is a client-side filter it is
already shaped to do.

**Why this matters more than tidiness: a remote host then needs no `ling-mem`
binary at all.** The CLI resolves through `daemon.json`, which can only
describe a *local* daemon — so CLI recall cannot reach another machine's
store, and a second machine would need its own binary, its own daemon and its
own store, which forks memory. Over MCP it needs a URL and a token.

Consequence for `autostart.sh`: installing and starting the binary should be
skippable on a host that only *reads* a remote store.

### `memory_*` is REMOVED from ling's `/mcp`

One memory service, not two. The tools come from ling-mem, so ling-mem is
where they are served — a host that wants memory adds ling-mem, a host that
wants the browser adds ling.

A proxy on the engine was considered and rejected: with the plugin shipping
both servers, a proxied `memory_search` and a direct one appear as two
distinguishable tools with identical schemas, and the model picks between
them arbitrarily. That is exactly the duplication `mcp-spec.md` warned about,
self-inflicted.

**Breaking change, deliberately.** `memory_*` has been on the engine's
`/mcp` since 1.4.0 (2026-07-10). Anyone who wired the engine's front door for
memory must add the second server. Keep the group for a deprecation window
with a notice rather than cutting it dead.

**This reverses the 2026-07-10 decision in `mcp-spec.md`** ("one MCP for all
users … two servers offering the same memory tools would confuse anyone
migrating"). The reversal belongs on that page too — a doc that still asserts
the opposite is the hidden-state failure, not a stale comment.

### Where the dispatch fixes live

`apply_dispatch_fixes` exists twice; ling-mem's copy says it is "ported from"
the engine's `memory_tool.rs`. The right single home is **ling-mem's MCP
layer** — those fixes exist because *models* fill arguments in sloppily
(`until: ""`, empty arrays narrowing to nothing). CLI arguments come from
clap, typed, from a human; REST does not need them.

Once the proxy lands, the engine's copy has no callers left — auto-recall
having moved to MCP too — so it is a deletion, not a migration.

### Migration

Mechanical, but it touches a lot: `Memory_query` / `Memory_write` become
`memory_search` / `memory_add` / … across `agents/*.md` tool lists,
`prompts/system-prompt.toml`, `SKILL.md`, and CFO's `allowed-tools`.

And `[agent].ling_mem_url` must be **resolved, not left**. After phase 2 its
only consumer was auto-recall, which now goes through `[mcp_servers]`. If it
is dead, delete it — never leave a knob with no consumer.

## Remote memory — a second machine on the LAN

A second machine's `ling` reaches the store by being an MCP client of
**ling-mem directly**:

```
DS242 ling ──LAN──► Mac ling-mem :9528/mcp   (x-linggen-device)
```

LAN is the scope. No relay, no VPN, no tunnel, no proxy hop.

**The auth is not new.** The engine's LAN gate is small — loopback passes,
`/api/health` and `/api/pair/*` pass, everything else needs an
`x-linggen-device` header matching a `secret` in
`~/.linggen/paired-devices.json` (`server/mod.rs`, `api/pair::is_valid_device_token`).
Both daemons live on the same machine under the same `~/.linggen`, so
**ling-mem validates against the same file**. Pair a machine once, through
the engine's existing screen-confirm flow, and the token works for both.

Safe defaults, so nobody exposes a biography by accident:

- ling-mem binds **loopback unless explicitly told otherwise**.
- Binding non-loopback **requires the token file to exist**; refuse to start
  otherwise rather than serve the store open on the Wi-Fi.
- A memory-only user never binds wide, so never needs the gate at all.

**This kills the `/api/memory/*` gateway** proposed earlier in
`network-spec.md`. Do not build it.

## What is deliberately not decided

- **Hub mode** — republishing every configured server on ling's `/mcp`.

## Sequencing

Phase 1 pays off the day it lands: GitHub, Playwright, Sentry, Figma and
every other MCP server become reachable from Linggen. Phase 2 is a follow-on.
Bundled together, a memory migration blocks a capability that stands on its
own.

Collapsing `apply_dispatch_fixes` to one copy is **prep for phase 2**, not
follow-up: the engine should not start depending on ling-mem's MCP while that
layer is still one of two.
