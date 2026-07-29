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
them. Every edge below is in the shipped code; the transport on each arrow is
what actually runs, not what could.

```mermaid
flowchart LR
  subgraph agents["Coding agents on this machine"]
    cc["Claude Code<br/>linggen plugin"]
    codex["Codex<br/>linggen plugin"]
    hooks["plugin hooks<br/>autostart.sh · recall.sh"]
    skill["ClawHub skill<br/>shared-memory"]
  end

  subgraph mac["The user's Mac"]
    ling["ling · engine<br/>0.0.0.0:9527"]
    lingmem["ling-mem · memory<br/>127.0.0.1:9528"]
    cli["ling-mem CLI"]
    shell["Linggen.app shell"]
    webui["Web UI<br/>127.0.0.1:9527"]
    dash["Data Browser<br/>127.0.0.1:9528/"]
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

  providers["LLM providers<br/>chatgpt · anthropic · openai<br/>gemini · deepseek"]

  cc -->|mcp / http| ling
  codex -->|mcp / http| ling
  hooks -->|spawn| cli
  skill -->|"Bash ling-mem verb"| cli
  cli -->|http| lingmem
  ling -->|http · /api/memory| lingmem
  lingmem -.->|"/mcp — built, unused by design"| lingmem

  shell -->|http · health keepalive| ling
  shell -->|webrtc · whip| ling
  webui -->|webrtc · whip| ling
  dash -->|http| lingmem
  ext -->|websocket · /api/bridge/socket| ling

  mobile -->|"webrtc · LAN whip"| ling
  mobile -->|https| llmapi
  relay -->|sdp offer| ling
  mobile -->|"webrtc · relay"| relay

  ling -->|https| providers
  ling -->|http| ollama
  ling -->|https · register + poll| relay
```

## The two daemons

| | `ling` (engine) | `ling-mem` (memory) |
|:--|:--|:--|
| Binds | `0.0.0.0:9527` | `127.0.0.1:9528` |
| Address from | `~/.linggen/config/linggen.runtime.toml` `[server]` | compiled `daemon::DEFAULT_PORT`, or `--port` |
| Publishes where it bound | — | `~/.linggen/memory/linggen-memory/daemon.json` |
| Started by | `Linggen.app` shell (LaunchAgent), `--idle-shutdown-secs 300` | `ling-mem start`, plugin autostart, or first CLI use |
| Outbound | LLM providers, `linggen.dev` relay | none — embeddings run in-process |

The asymmetry is the source of most address bugs: **the one with a config file
publishes no discovery, and the one with discovery has no config file.** A
client of `ling` must guess its port; a client of `ling-mem` can read it but
cannot point it elsewhere.

## Edges

**Into `ling` (`:9527`)**

| From | Transport | Path |
|:--|:--|:--|
| Claude Code / Codex plugin | HTTP MCP, JSON-RPC | `/mcp` |
| Linggen.app shell | HTTP | `/api/health`, every 60s while a window is open |
| Shell, Web UI, phone on LAN | WebRTC | `/api/rtc/token` → `/api/rtc/whip` |
| Phone off LAN | WebRTC over relay | `linggen.dev` signalling → `/api/signaling/<nonce>/answer` |
| linggen-browser extension | WebSocket, extension dials in | `/api/bridge/socket` |
| Skills reaching the browser | HTTP | `POST /api/bridge/call` |

**Into `ling-mem` (`:9528`)**

| From | Transport | Path |
|:--|:--|:--|
| `ling` | HTTP REST | `POST /api/memory/<verb>`, URL from `[agent].ling_mem_url` |
| `ling-mem` CLI | HTTP REST | port read from `daemon.json` |
| Plugin hooks | spawn the CLI | `ling-mem search`, `days`, `status`, `start` |
| ClawHub `shared-memory` skill | spawn the CLI | `Bash ling-mem <verb>` — every op, no exception |
| Browser | HTTP | `/` — the Data Browser UI |

### Four front doors to one store

| Caller | Route | Needs the engine? |
|:--|:--|:--|
| Outside agent via the plugin | `/mcp` → `call_memory_http` → REST | yes |
| Linggen's own agents | `Memory_query` / `Memory_write` → REST | yes |
| ClawHub skill, plugin hooks, any agent with Bash | `ling-mem` CLI → REST | **no** |
| `ling-mem`'s own `/mcp` | direct | no — but unused, see below |

The CLI route is the engine-free channel and the reason it exists: a ClawHub
user installs the skill, the skill installs the binary (`install-bin.sh
--version '^1'`), and memory works with no `ling` on the machine at all. The
same `SKILL.md` also lists `Memory_query`/`Memory_write` in `allowed-tools`,
but that block is Linggen-only — "Claude Code / Codex ignore this block" — so
one skill file takes the engine route on Linggen and the CLI route everywhere
else.

Worth holding next to the `mcp-spec.md` line that the engine is the base
install for every channel: for the ClawHub channel today, it isn't.

**Out of the Mac**

| From | To | Why |
|:--|:--|:--|
| `ling` | provider APIs, `linggen.dev/api/llm`, local Ollama | inference |
| `ling` | `linggen.dev` | register the instance, poll for SDP offers |
| Phone | `linggen.dev/api/llm`, `/api/search` | Yinyue's model and web search |

Everything else the phone does — skills, memory, DJ files, photos, Ling's
chat — is tunnelled inside the WebRTC data channel as `http_request`, so it
works identically on the LAN and over the relay.

## One MCP, on purpose

`ling-mem` serves its own `/mcp` with 13 tools, and **nothing connects to it.**
That is a decision, not an oversight — see `mcp-spec.md` (2026-07-10): one MCP
front door for every channel, because two servers offering the same memory
tools confuses migrating users, and a memory-only install can't run the dream
missions. The engine is the base install; `ling-mem` is the component it
manages and proxies.

Consequence: memory reaches an outside agent by the long route —
`agent → ling /mcp → HTTP REST → ling-mem` — and the dispatch-normalisation
step exists in both repos (`ling-mem` `src/http/mcp.rs` says its
`apply_dispatch_fixes` are "ported from" the engine's `memory_tool.rs`).

## Where an address comes from, today

| Consumer | Reads | Overridable |
|:--|:--|:--|
| `ling` server bind | `linggen.runtime.toml` `[server]` | yes |
| plugin `.mcp.json` | hardcoded `127.0.0.1:9527` | **no** |
| plugin `autostart.sh` | `LINGGEN_PORT`, default `9527` | env only |
| `ling` → `ling-mem` | `[agent].ling_mem_url` | yes |
| `ling` `cli/status.rs` | `DEFAULT_LING_MEM_PORT` const | **no** — ignores the above |
| `ling` permission check | hardcoded `127.0.0.1:9528` | **no** |
| `ling-mem` CLI | `daemon.json` | n/a — discovery |
| `linggen-vscode` | its own `linggen.dashboard.port` | yes |

`linggen/src/config.rs` carries `DEFAULT_LING_MEM_PORT` with a comment saying
it must match `linggen-memory`'s `daemon::DEFAULT_PORT` — a constant
hand-synced across two repos. The 2026-07 migration from `9898` proved the
hazard: `.mcp.json` was flipped and `autostart.sh` was not, so every session
started a second daemon on a port nothing served, and both registered the same
relay instance and split the phone's traffic.

## Open

**One source of truth for addresses.** Resolution order for every consumer:
env → a shared `endpoints.toml` → the daemon's own discovery file → compiled
default. `ling` should write a discovery file the way `ling-mem` already does,
`.mcp.json` should use `${LINGGEN_PORT:-9527}` expansion (supported in `url`),
and the hardcoded constants should read rather than restate.

**Remote endpoints.** A second machine (e.g. DS242) using the Mac's memory
cannot be solved by discovery — a remote address has to be stated. Two shapes,
undecided: through the engine's already-authenticated `/mcp` (consistent with
the one-front-door decision, reuses device tokens and the relay), or `ling-mem`
learning to bind and authenticate (keeps it standalone, but reopens a decision
already made, and adds a second auth surface to secure).

**Two daemons, one relay instance.** Both registrations are accepted, and the
phone's offers are answered by whichever polls first. The second should be
refused.
