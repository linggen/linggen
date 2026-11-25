---
type: spec
reader: Coding agent and users
guide: |
  Product specification — describe what the system should do and why.
  Keep it brief. Aim to guide design and implementation, not document code.
status: BUILT, complete. Phase 1 2026-07-29 (client, tool surface, project scope, McpTab); phase 2 2026-07-30 (built-in memory server, auto-recall over MCP, tool-name migration, LAN gate, recall.sh over MCP). 2.5: the plugin's two servers became one door (`mcp-spec.md`); the sections that describe the two-server layout are marked as history. Unproven: a real second machine pointed at the store.
---

# Ling as an MCP client

Linggen could be *driven by* MCP and not *consume* it: `src/server/mcp/` and
`src/server/mcp_agent.rs` were both servers, with no outbound `tools/list`
anywhere in the tree.

Every peer — Claude Code, Codex, Cursor, Cline, Zed — connects to arbitrary
MCP servers. Linggen connected to none. For a product whose premise is an OS
for agents, that was an OS that could not load third-party drivers. **Both
phases are built** (`src/mcp_client/`); what follows is the design as it
shipped, with what remains marked in place.

## A model is needed to CHOOSE a tool, not to CALL one

This is the load-bearing fact for everything below, and it is easy to get
wrong (this spec's first draft did). MCP is JSON-RPC. The client is a
program. Verified against the running daemon with nothing but curl — no
agent, no model, no `initialize` handshake, because the server is stateless:

```
POST 127.0.0.1:9527/mcp
{"jsonrpc":"2.0","id":1,"method":"tools/call",
 "params":{"name":"memory_search","arguments":{"query":"…","limit":2}}}
→ 2 rows
```

So anything in the engine can call an MCP tool, including code paths that run
before the model. That is what makes auto-recall over MCP possible, and it
removes the last reason for a separate memory address (2.5: memory is on the
one door, `/mcp`).

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

Memory was **Chat-tier — ungated, no prompts** as two built-in tools, and
stayed ungated as a server (its entry declares `gated: false`, so nothing in
`engine/permission/model.rs` knows its name). A user-added MCP server must not
be: its tools can write files and call APIs, which is precisely what
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

## Phase 2 — ling-mem as the built-in server — BUILT 2026-07-30

Memory stopped being a hand-built special case (`engine/tools/memory_tool.rs`,
`Memory_query` / `Memory_write`) and became the MCP server that ships
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
     { query: <prompt>, scope_root: <root or skill dir>, limit: K }
3. → the memory server in [mcp_servers]
4. filter by min_score, inject top-K
5. model runs
```

Local and remote are the same code with a different URL. Two details:

- **`scope_root`** (`cwd_scope` on a pre-v2 daemon) carries the recall scope: an owner session's root, or a
  memory-using skill's own dir (`~/.linggen/skills/<name>`), which is what
  keeps CFO to its own rows. (`contexts` scoping retired 2026-10-04.)
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

`recall.sh` already depends on `jq`, and rows carry `hybrid_score` and `score`,
so the missing `min_score` is a client-side filter it is
already shaped to do.

**Why this matters more than tidiness: a remote host then needs no `ling-mem`
binary at all.** The CLI resolves through `daemon.json`, which can only
describe a *local* daemon — so CLI recall cannot reach another machine's
store, and a second machine would need its own binary, its own daemon and its
own store, which forks memory. Over MCP it needs a URL and a token.

Consequence for `autostart.sh`: installing and starting the binary should be
skippable on a host that only *reads* a remote store.

### One door for memory and the rest (2.5)

Memory and Linggen's other capabilities are one MCP server, `linggen`, at
`/mcp`: one catalogue of tools, five of them memory (`mcp-spec.md`). A host adds one
server; there is no second memory server and no second `memory_search`.

The engine's own agents are a different caller. They reach memory through the
built-in memory server at `/mem/mcp` (this machine only), which publishes the
five memory tools and is not for hosts.

**History.** From 2026-07-30 to 2.4 the engine's `/mcp` did not carry memory
and the plugin shipped two servers (24 + 17 tools), because a proxied
`memory_search` beside a direct one would have been two identical tools in
front of one model. 2.3 put memory's server on the engine's port as
`/mcp/memory`; 2.5 retired that address and merged the two servers. A plugin
that still points at `/mcp/memory` gets HTTP 410 `UPGRADE_PLUGIN` and is told
to update the plugin and restart the host; nothing is forwarded.

### Where the dispatch fixes live

`apply_dispatch_fixes` exists twice; ling-mem's copy says it is "ported from"
the engine's `memory_tool.rs`. The right single home is **ling-mem's MCP
layer** — those fixes exist because *models* fill arguments in sloppily
(`until: ""`, empty arrays narrowing to nothing). CLI arguments come from
clap, typed, from a human; REST does not need them.

Done: `linggen-memory` `3c3bb0a` collapsed them into ling-mem's MCP layer, and
the engine's copy went with the tools. Not ported deliberately: the engine's
`host: "linggen"` default. That is the *caller* naming itself, and a daemon
that also serves Claude Code, Codex and Cursor would mislabel their rows —
so each caller stamps its own (`engine/tools/memory_mcp.rs`).

### Migration — DONE 2026-07-30

Mechanical, and it touched a lot: `Memory_query` / `Memory_write` became
`memory_*` across `agents/*.md` tool lists, `prompts/system-prompt.toml`, the
dream mission, `SKILL.md`, and CFO's `allowed-tools`.

A declaration names the **server**, not each tool: `mcp__memory` in an agent's
`tools:`, a skill's `allowed-tools:` or a mission's expands to what that server
advertises, through one rule shared by prompt assembly and the scope sets
(`extensions/scope.rs`). Pulse is the exception — it reads the founder's
context and never curates it, so it names the read tool alone
(`mcp__memory__memory_search`: its semantic, get and list).

`[agent].ling_mem_url` is **gone** (2.3): the engine hosts the store, so the
built-in memory server's endpoint is its own `/mem/mcp` and the program-side
REST calls go to `/mem/api/memory/<verb>` on the same port
(`memory_host::url`). An old config's key is ignored.

Four things a raw pass-through would have lost, and where they went: the
calling host's name, the authoring session id, where the session stands
(`cwd`, `root`, `scope_root` — or `cwd_scope` on a pre-v2 daemon; a
memory-using skill stands in its own dir), and
the AskUser unlock on the user-voice guard. All four are filled in
client-side by `engine/tools/memory_mcp.rs`, each field only where the
server's own advertised schema declares it **for the operation the call
runs**: `/mem/mcp` publishes memory's five folded tools (`memory_search`
semantic/get/list, `memory_add`, `memory_update`, `memory_delete`,
`memory_upkeep` with the store's eleven upkeep ops — never `dream_status`,
`dream_run`, an agent or a browser tool), and a folded schema is the union
of its operations' fields, so the client reads the call's branch off
memory's catalogue (`Catalog::branch_of`) — `scope_root` goes on a search
or a list, never a get; `cwd` and `root` on an add or `session_start`,
never another upkeep op. The user-voice guard reads each target row with
`memory_search` op=get. The retired 2.4 tool names are unknown there: no
alias. That is argument shaping, not the proxy above
— nothing republishes ling-mem's tools.

## Remote memory — a second machine on the LAN

A machine with only a plugin reaches the store by being an MCP client of the
owner's Linggen:

```
second machine ──LAN──► Mac ling :9527/mcp   (x-linggen-device)
```

An approved computer sees the five memory tools only (`mcp-spec.md` § Views).
A machine with Linggen shares memory as a replica (`memory-spec.md` § Across machines). *History:* before 2.3 this was ling-mem's own `:9528/mcp`; nothing
listens on 9528 now.

LAN is the scope. No relay, no VPN, no tunnel, no proxy hop.

**The auth is not new.** The engine's LAN gate is small — loopback passes,
`/api/health` and `/api/pair/*` pass, everything else needs an
`x-linggen-device` header matching a `secret` in
`~/.linggen/paired-devices.json` (`server/mod.rs`, `api/pair::is_valid_device_token`).
Both daemons live on the same machine under the same `~/.linggen`, so
**ling-mem validates against the same file**. Pair a machine once, through
the engine's existing screen-confirm flow, and the token works for both.

Safe defaults, so nobody exposes a biography by accident — **built
2026-07-30** in `linggen-memory` `src/http/gate.rs`:

- ling-mem binds **loopback unless `--host` says otherwise**.
- Binding non-loopback **requires the token file to exist**; it refuses to
  start otherwise, checked before the listener exists so a refusal is never a
  port briefly open and unguarded.
- A non-loopback request must carry a paired device's token; loopback passes,
  and `/api/health` stays open as a probe.
- A memory-only user never binds wide, so never needs the gate at all.

**This kills the `/api/memory/*` gateway** proposed earlier in
`network-spec.md`. Do not build it.

## What is deliberately not decided

- **Hub mode** — republishing every configured server on ling's `/mcp`.

## What is left

**A real second machine.** Every piece is built — the plugin's one server entry
(`/mcp`), both hooks talk MCP over curl, `autostart.sh` installs nothing when
`LING_MEM_HOST` points elsewhere, and ling-mem's gate checks the token — but no
second machine has actually been pointed at this store. `LING_MEM_HOST` +
`LING_MEM_TOKEN`, paired once through Linggen, nothing installed locally. Until
that run happens it is designed and unit-true, not proven.

## Sequencing

Phase 1 pays off the day it lands: GitHub, Playwright, Sentry, Figma and
every other MCP server become reachable from Linggen. Phase 2 is a follow-on.
Bundled together, a memory migration blocks a capability that stands on its
own.

Collapsing `apply_dispatch_fixes` to one copy is **prep for phase 2**, not
follow-up: the engine should not start depending on ling-mem's MCP while that
layer is still one of two.
