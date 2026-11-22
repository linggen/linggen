---
type: spec
reader: Coding agent and users
guide: |
  Product specification — describe what the system should do and why.
  Keep it brief. Aim to guide design and implementation, not document code.
status: describes what ships today; the open section is not built
---

# Linggen network — who talks to whom

Two daemons on the user's machine, and everything else is a client of one of
them. **Since 2.3 (unreleased) memory is hosted by the engine** — one
process on one port; see `memory-spec.md` § Memory is hosted by the engine.
The diagram and tables below still show the two-daemon edges; the rows marked
*2.3* say what answers them now. Nothing listens on 9528 any more: a caller
that still names it (a plugin before its move to 9527) gets no answer. Every edge below is in the shipped code; the transport on each arrow is
what actually runs, not what could. What is decided but not yet built lives in
`## Open`, never in the diagram — an arrow that doesn't run is a phantom.

```mermaid
flowchart LR
  subgraph agents["Coding agents on this machine"]
    cc["Claude Code<br/>linggen plugin"]
    codex["Codex<br/>linggen plugin"]
    hooks["plugin hooks<br/>autostart.sh · recall.sh"]
  end

  subgraph mac["The user's Mac"]
    ling["ling · engine<br/>0.0.0.0:9527"]
    lingmem["ling-mem · memory<br/>127.0.0.1:9528"]
    cli["ling-mem CLI"]
    shell["Linggen.app shell"]
    webui["Web UI<br/>127.0.0.1:9527"]
    dash["Settings → Memory<br/>rows · days · review queue"]
    ext["Chrome +<br/>linggen-browser"]
    ollama["Ollama<br/>127.0.0.1:11434"]
  end

  subgraph phone["iPhone · Linggen Mobile"]
    mobile["Linggen Mobile"]
  end

  subgraph cloud["linggen.dev"]
    relay["relay + signalling"]
    llmapi["/api/llm<br/>/api/search"]
  end

  third["third-party MCP servers<br/>github · playwright · sentry"]
  providers["LLM providers<br/>chatgpt · anthropic · openai<br/>gemini · deepseek"]

  cc -->|"mcp — /mcp, thirteen tools"| ling
  codex -->|"mcp — /mcp, thirteen tools"| ling
  hooks -->|"curl · mcp tools/call"| ling
  cli -->|http| lingmem

  ling -->|"mcp client — tools + auto-recall"| lingmem
  ling -->|"mcp client"| third
  ling -->|"http · dream · core block · skill pages"| lingmem

  shell -->|http · health keepalive| ling
  shell -->|webrtc · whip| ling
  webui -->|webrtc · whip| ling
  dash -->|webrtc| ling
  ext -->|websocket · /api/bridge/socket| ling

  mobile -->|"webrtc · LAN whip"| ling
  mobile -->|"webrtc · POST /api/memory/*"| ling
  mobile -->|https| llmapi
  relay -->|sdp offer| ling
  mobile -->|"webrtc · relay"| relay

  ling -->|https| providers
  ling -->|http| ollama
  ling -->|https · register + poll| relay
```

**`ling` is an MCP client** (shipped 2026-07-30). It consumes any server the
user adds, and reaches memory the same way — `ling-mem` is the built-in one,
merged into `[mcp_servers]` at connect time and declared `gated: false`, so
nothing in the permission code knows its name. Auto-recall rides the same
client even though it fires *before* the model: a tool **call** is a JSON-RPC
request, and making one needs a program, not a model.

The engine still speaks REST to `ling-mem` where it is a program rather than an
agent — the dream rollup, the mission scheduler's stats, the core block,
`/apps/<skill>/capability/*` for a skill webpage's own clicks (which reach
`ling` over the page's data channel), and the phone's
`POST /api/memory/<verb>` passthrough. *2.3:* none of these is a request any
more — the engine calls the store it hosts in-process, on memory's own
runtime, and gets the answer the route would give.

**The phone reaches memory through `ling`, and must** (2026-07-30). Yinyue is
resident on the phone: her tool loop runs there, so she is the one caller that
needs this store from off-machine — and off the LAN there is no IP route to the
Mac at all, only `http_request` frames on the WebRTC channel, which terminate in
the engine's router. So `POST /api/memory/<verb>` forwards to
`[agent].ling_mem_url` (*2.3:* calls the hosted store in-process) and returns
ling-mem's envelope whole. One route covers
all eighteen verbs; ling-mem stays the authority on which exist. The phone never
learns a port — moving `ling-mem` is a Mac-side config change and nothing else.

This is not the tool proxy cut from `/mcp` the same day. That put a second copy
of ling-mem's tools in front of a model, which then had to guess between them.
This declares no tool: no schema, no dispatch translation, one verb segment
forwarded. It is transport, like the tunnel it rides on. And the phone takes the
REST door for the reason above — three fixed verbs, choosing nothing, a program
rather than an agent.

The phone's Ling tab needs none of it: it drives a session on the Mac, and that
agent reaches memory on its own loopback like every local caller.

Each tool is served in exactly one place: since 2.5, `/mcp` on the engine's
port (memory, agents, browser, apps — thirteen tools, `mcp-spec.md`).

## The two daemons

| | `ling` (engine) | `ling-mem` (memory) |
|:--|:--|:--|
| Binds | `0.0.0.0:9527` | `127.0.0.1:9528` — loopback unless `--host`, which is refused without paired devices. *2.3:* nothing; memory is on the engine's port |
| Address from | `[server].url` in `~/.linggen/config/linggen.runtime.toml` | compiled `daemon::DEFAULT_PORT`, or `--port`. *2.3:* the engine's |
| Publishes where it bound | — | `~/.linggen/memory/linggen-memory/daemon.json`. *2.3:* nothing; the store lock `memory/.store.lock` names the holder's pid and engine port |
| Started by | `Linggen.app` shell (LaunchAgent); stays up once started — it hosts memory (the old `--idle-shutdown-secs` flag is ignored) | `ling-mem start`, plugin autostart, or first CLI use. *2.3:* not a process — the engine opens the store at start-up and stops an old daemon first |
| Outbound | LLM providers, `linggen.dev` relay | none — embeddings run in-process |

The asymmetry is the source of most address bugs: **the one with a config file
publishes no discovery, and the one with discovery has no config file.** A
client of `ling` must guess its port; a client of `ling-mem` can read it but
cannot point it elsewhere.

## Edges

**Into `ling` (`:9527`)**

| From | Transport | Path |
|:--|:--|:--|
| Claude Code / Codex plugin | HTTP MCP, JSON-RPC | `/mcp` — the one door: memory (5), agents (4), browser (3); the view depends on the caller (`mcp-spec.md` § Views) |
| Linggen.app shell | HTTP | `/api/health`, every 60s while a window is open |
| Shell, Web UI, phone on LAN | WebRTC | `/api/rtc/token` → `/api/rtc/whip`; then every `/api/*` call as `http_request` on the channel — the web UI has no HTTP data path (webrtc-spec.md) |
| Skill pages (`/apps/<skill>/…`) | WebRTC | the same, through `/shared/channel.js`: the desktop app's window's channel when its tabs frame them, else a hidden `/?channel=1` frame's own peer; only their files are HTTP (webrtc-spec.md § Skill pages) |
| Phone off LAN | WebRTC over relay | `linggen.dev` signalling → `/api/signaling/<nonce>/answer` |
| linggen-browser extension | WebSocket, extension dials in | `/api/bridge/socket` |
| Skill scripts reaching the browser | HTTP | `POST /api/bridge/call` (a skill page's own call rides its channel) |
| Phone, for Yinyue's memory | HTTP in the tunnel | `POST /api/memory/<verb>` → `ling-mem`. *2.3:* the hosted store, scoped to the phone's person |
| *2.3:* another machine, approved (`machine` token) | HTTP | `/mcp` — the five memory tools (a plugin-only machine); a replica: `/mem/api/sync/*` and its pushed writes |
| `/mcp/memory` (retired in 2.5) | HTTP | answers 410 `UPGRADE_PLUGIN`; nothing is forwarded |
| *2.3:* a machine asking to connect | HTTP, no token | `/mem/api/connect/request`, `/mem/api/connect/status/<id>` |

**Into `ling-mem` (`:9528`)**

| From | Transport | Path |
|:--|:--|:--|
| `ling`, on behalf of a model | HTTP MCP, JSON-RPC | `/mcp` — the built-in server, URL from `[agent].ling_mem_url` + `/mcp`. *2.3:* the hosted store, `/mem/mcp` on the engine's own port; no address to set |
| `ling`, as a program | HTTP REST | `POST /api/memory/<verb>` — dream rollup, scheduler stats, core block, skill pages, the phone's passthrough. *2.3:* no edge — an in-process call on memory's runtime |
| A second machine on the LAN | HTTP MCP, JSON-RPC | `/mcp` with `x-linggen-device`. *2.3:* not on the network — the engine's port, `/mcp/memory` (retired in 2.5); *2.5:* `/mcp` |
| `ling-mem` CLI | HTTP REST | port read from `daemon.json`. *2.3:* `ling mem <verb>` — the engine's port from the lock file, `/mem/api/memory/<verb>`; engine down, the store directly under the lock |
| Plugin hooks | HTTP MCP, JSON-RPC | `memory_search` per turn, `memory_search op=list` for core, `memory_upkeep op=days` for upkeep. *2.3:* the engine's port, `/mcp/memory` (retired in 2.5); *2.5:* `/mcp`; 9528 no longer answers |
| Plugin hooks (local only) | spawn the CLI | `ling-mem status`, `start` — daemon lifecycle, skipped on a remote host. *2.3:* `~/.local/bin/ling-mem` is the engine's stand-in: `status` says running at the engine's version, `start`/`restart` do nothing |
| Linggen UI | WebRTC | Settings → Memory — rows, days and review queue; scan via `/linggen:scan` in the Claude Code / Codex plugin |

### Four front doors to one store

| Caller | Route | Needs the engine? |
|:--|:--|:--|
| Linggen's own agents | `mcp__memory__memory_*` → hosted `/mem/mcp` | yes |
| Outside agent via the plugin | engine `/mcp` (the five memory tools among thirteen) | yes |
| Plugin hooks, any agent with Bash | `ling mem` CLI → hosted store, or direct under lock | no |
| A second machine's agents | engine `/mcp` + `x-linggen-device` (the five memory tools only) | yes |

Any MCP-capable agent can add `http://127.0.0.1:9527/mcp` as an
MCP server. A plugin-only second machine needs the engine's URL
and a device token, not a store of its own.

**Out of the Mac**

| From | To | Why |
|:--|:--|:--|
| `ling` | provider APIs, `linggen.dev/api/llm`, local Ollama | inference |
| `ling` | `linggen.dev` | register the instance, poll for SDP offers |
| `ling` | `linggen.dev/api/saves`, `/api/meters` | a skill's declared cloud save and token meter (skill-spec § Cloud) |
| `ling` | GitHub (our repos), then `linggen.dev/dl/*` | self-update, Python runtime, built-in skills; the mirror only on connect/timeout/5xx (`src/mirror.rs`) |
| `ling` | `registry.npmjs.org/@openai/codex/latest`, else GitHub's latest `openai/codex` release | the Codex client version presented to ChatGPT — a plain GET with no token, only while signed in to ChatGPT, about once a day and at sign-in (`provider/models/chatgpt_catalog`) |
| `ling` | the ChatGPT backend's `models?client_version=` | the account's model list, read after the version; a family moves to a newer listed model only after one tiny request on it answers |
| Phone | `linggen.dev/api/llm`, `/api/search` | Yinyue's model and web search |

Everything else the phone does — skills, memory, DJ files, photos, Ling's
chat — is tunnelled inside the WebRTC data channel as `http_request`, so it
works identically on the LAN and over the relay. Memory was the exception for
three days: it addressed `/mcp` for tools the engine had stopped serving, and
failed silently because the phone reads a missing `result` as "no Mac". Fixed
2026-07-30 by the passthrough above.

## Memory over MCP — one door (2.5)

Memory is served on the engine's `/mcp` with the engine's other tools: thirteen
tools, five of them memory. The engine's own agents use the built-in memory
server (`/mem/mcp`, this machine only), the plugin's Claude Code / Codex
sessions use the one `linggen` server entry, the plugin's hooks use curl, and a
second machine on the LAN uses `/mcp` with its device token and sees the five
memory tools only. `/mcp/memory` answers 410 `UPGRADE_PLUGIN` (a plugin that
still points there is told to update and restart its host); nothing is
forwarded.

*History.* Memory's own MCP server (`ling-mem`'s `/mcp`, built 2026-05-27) was
the promoted memory endpoint from 2026-07-29 to 2.2, beside the engine's `/mcp`,
so a plugin had two server entries. 2.3 moved it onto the engine's port as
`/mcp/memory` (retired in 2.5); 2.5 merged the two servers into one (`mcp-spec.md` § History).

The dispatch-normalisation step lives in one place, memory's MCP layer
(`crates/linggen-memory/src/http/mcp.rs`). Those fixes exist because *models*
fill arguments in sloppily (`until: ""`, empty arrays narrowing to nothing);
CLI arguments come from clap, typed, from a human, and REST callers are
programs.

## Where an address comes from, today

Two different questions, and conflating them is the bug that keeps recurring:
**serve at** is a machine's declaration about its own daemon; **connect to** is
a host's declaration about where it goes looking. On the machine that runs
everything they coincide, which is exactly what makes them easy to merge by
mistake — on a second machine they are facts about different computers.

**Serve at**

| Daemon | Reads | Notes |
|:--|:--|:--|
| `ling` | `[server].url` in `linggen.runtime.toml` | `--port` / `LINGGEN_PORT` override; legacy `host`+`port` still parse and say so at startup |
| `ling-mem` | `--port` / `--host` | no config file; publishes `daemon.json`. *2.3:* gone; memory is `/mem` and `/mcp` on the engine's port (this machine; an approved machine reaches `/mcp` — the memory tools — and the replica protocol); nothing on 9528 |

Nothing else asserts a bind address. The app shell and the plugin's autostart
both **read** `[server].url` and no longer pass `--port` — they used to assert
their own default, which is how a user's `[server].port` could be set and
ignored, and how the 9898 stray daemon was launched.

**Connect to**

| Client | Reads | Notes |
|:--|:--|:--|
| plugin hooks | `~/.linggen/client.json` | env > file > default, via `hooks/mcp.sh` |
| plugin `.mcp.json` | `${LINGGEN_HOST/PORT}`, `${LING_MEM_HOST/PORT}` | CC expands at startup, **before** any hook — so it can only take env; `config.sh` mirrors `client.json` into `settings.json` `env` |
| `ling` → `ling-mem` | `[agent].ling_mem_url` > `$LING_MEM_URL` > `:9528` | *2.3:* gone — always the store the engine hosts (`memory_host::url`); an old config's key is ignored |
| `ling` `cli/status.rs` | the resolved `[agent].ling_mem_url` | *2.3:* the store lock's port |
| `ling-mem` CLI | `daemon.json` | discovery; local only by construction. *2.3:* `ling mem` reads `memory/.store.lock` |
| `linggen-vscode` | its own `linggen.dashboard.port` | yes |

`linggen/src/config.rs` carried `DEFAULT_LING_MEM_PORT` (gone in 2.3), a
constant hand-synced with `linggen-memory`'s `daemon::DEFAULT_PORT`. The 2026-07 migration from `9898` proved the
hazard: `.mcp.json` was flipped and `autostart.sh` was not, so every session
started a second daemon on a port nothing served, and both registered the same
relay instance and split the phone's traffic.

## Open

**Discovery.** `ling` still publishes no discovery file the way `ling-mem`
does — which is what a client would read to find a *running* daemon rather
than a configured one. (*2.3:* the store lock `memory/.store.lock` names the
engine's pid and port; `ling status` and `ling mem` read it. No `9528` is
left in `src/`.)

**A second machine, end to end.** *2.3:* machines share memory as primary
and replicas, each Linggen with its own store; a machine with only a plugin
points it at another machine's `/mcp` with that machine's token. Both
paths run between two engines on one Mac over its LAN address; nobody has yet
done it from a real second machine. The plugin lives in this repo
(`plugins/linggen`); its entries and hooks use the engine's port, and an
older install that still names 9528 gets no answer. A replica that saved a primary at `host:9528` pairs again with
`host:9527`.

**Two daemons, one relay instance.** Both registrations are accepted, and the
phone's offers are answered by whichever polls first. The second should be
refused.
