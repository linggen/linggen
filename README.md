<p align="center">
  <img src="logo.svg" width="120" alt="Linggen" />
</p>

<h1 align="center">Linggen</h1>

<p align="center">
  <strong>Local-first AI apps on your own machine.</strong><br>
  A personal CFO that reads your bank statements without uploading them, a Mac
  health doctor, a music DJ — full apps on one open runtime, with a
  general-purpose assistant built in.
</p>

<p align="center">
  <a href="https://linggen.dev">Website</a> &middot;
  <a href="https://linggen.dev/#inside">Apps</a> &middot;
  <a href="https://linggen.dev/docs">Docs</a>
</p>

<p align="center">
  <a href="https://github.com/linggen/linggen/releases"><img src="https://img.shields.io/github/v/release/linggen/linggen?style=flat-square" alt="Release" /></a>
  <a href="https://github.com/linggen/linggen/blob/main/LICENSE"><img src="https://img.shields.io/github/license/linggen/linggen?style=flat-square" alt="Apache 2.0 License" /></a>
  <a href="https://github.com/linggen/linggen/stargazers"><img src="https://img.shields.io/github/stars/linggen/linggen?style=flat-square" alt="Stars" /></a>
</p>

<p align="center">
  <img src="doc/media/demo.gif" width="800" alt="Linggen — CFO catches a double charge in locally-parsed statements, Sys Doctor scores your Mac's health, DJ runs karaoke with synced lyrics" />
</p>
<p align="center">
  <sub><strong>CFO</strong> catches a $389 double charge (statements never leave your Mac) · <strong>Sys Doctor</strong> scores your Mac's health · <strong>DJ</strong> runs karaoke night. <a href="https://linggen.dev">Full demo →</a></sub>
</p>

---

## Install

**The Mac app** — launcher with all apps below, menubar companion included:

```bash
curl -fsSL https://linggen.dev/install-app.sh | bash
```

**Engine only** — CLI + web UI, macOS and Linux:

```bash
curl -fsSL https://linggen.dev/install.sh | bash
ling
```

Opens the web UI at `http://localhost:9898`.

---

## The apps

- **CFO** — a personal CFO that never uploads your money data. Drop in bank/card CSV or PDF exports: deterministic local code parses them into a spend report — monthly trends, subscriptions, commitments, transfer detection, duplicate-charge checks. The AI layer only explains and reviews, and account numbers are stripped before it sees even the redacted totals. [60-second demo](https://linggen.dev/cfo-demo.mp4).
- **Sys Doctor** — AI health analyst for your Mac: disk, security, performance, dormant apps, buyer's guide. Recommends; never acts on its own.
- **DJ** — tell it a vibe and it curates a set, builds your local music library with clean tags, syncs tracks to your phone, and does karaoke with synced, translated lyrics.
- **Pulse** — GTM brain for solo founders. Reads your configured trends and feeds, then drafts on-voice posts and replies for X and Reddit.
- **Games** — Chinese Chess and Gomoku where the model actually plays you, plus Snake, Pong, and Tetris.
- **Memory** — cross-agent semantic memory via [`ling-mem`](https://github.com/linggen/linggen-memory): the same store reachable from Linggen, Claude Code, or Codex.
- **Model sharing** — open a room and let friends use your models over P2P WebRTC. No keys for the consumer, no cloud middleman; the owner controls budget and tools.

Skills, agents, missions — all files. New apps are a folder away. Browse community skills at [github.com/linggen/skills](https://github.com/linggen/skills).

---

## What is Linggen?

Architecturally, Linggen is **the root system for AI agents**. The core
runtime manages agent processes, communication, and execution; everything
else (skills, agents, missions) grows on top as files. An "AI app" in
Linggen is a skill, an agent, or a mission — markdown + scripts, not code
plugins. The runtime gives every app a process, syscalls (built-in tools),
a filesystem (memory), permissions, and a network surface (P2P rooms).

Apps drop into a folder and run.

### OS analogy

| OS | Linggen |
|:---|:---|
| Process | Agentic loop — one running agent |
| Interrupt | User message queue — checked each iteration |
| Thread / Fork | Subagent delegation — concurrent child execution |
| Syscall | Tool call — built-in tools are the kernel API |
| Dynamic library | Skill — loaded at runtime, no code changes |
| Cron job | Mission — scheduled agent / app / script |
| Driver | Model provider — Ollama, Claude, GPT, Gemini, Bedrock |
| Filesystem | Memory store — core markdown + LanceDB RAG via `ling-mem` |
| Process privilege | Permission modes (chat / read / edit / admin) + path scoping |
| Network share | Rooms — share models with peers over P2P WebRTC |

Full table and design principles in [`doc/product-spec.md`](doc/product-spec.md);
vision and roadmap in [`doc/insight.md`](doc/insight.md).

---

## Yinyue — your desktop companion

A VRM avatar and conversational companion built into the runtime — the face the
agents wear. She reads the room (whether you're typing, reading, or away), voices
the moments that matter in her own words, and gives the agents a way to talk to
each other and to you.

- **She heralds your agents.** When one is blocked waiting on you, fails, or
  finishes a background job, Yinyue says so — and stays quiet on the routine.
- **Relay an answer.** When an agent is parked on a question or a permission,
  just tell Yinyue "approve" — she carries your word back and it unblocks.
- **`agent_chat`.** Agents message each other. Ask Yinyue to have Ling introduce
  itself and the message lands in your chat (`[Yinyue]: …`) and Ling replies;
  tell Ling to make Yinyue dance and she dances. A loop-break keeps you in the loop.
- **She's present.** Gestures and moods (`Express`), occasional unprompted
  remarks, a local voice with lip-sync.

She's an ordinary Linggen session on the `yinyue` agent — swap her model, edit
her persona, or build another companion the same way. See
[`doc/yinyue-companion-spec.md`](doc/yinyue-companion-spec.md).

---

## Add an app

Drop a markdown file in `~/.linggen/` — available immediately, no restart:

```markdown
---
# ~/.linggen/agents/reviewer.md
name: reviewer
description: Code review specialist.
tools: ["Read", "Glob", "Grep"]
model: claude-sonnet-5
---

You review code for bugs, style issues, and security vulnerabilities.
```

Skills (`~/.linggen/skills/<name>/SKILL.md`) and missions (cron-scheduled
agent / app / script) follow the same drop-in pattern. Skills use the open
[Agent Skills](https://agentskills.io) standard and work in Claude Code
and Codex too.

---

## Where Linggen sits

- **Local-first.** Runtime, data, and inference (when you pick local models) live on your machine. Cloud is opt-in via your own API keys.
- **Model-agnostic.** Any model — Ollama, Claude, GPT, Gemini, DeepSeek, Groq, OpenRouter. Routing policies (`local-first`, `cloud-first`, custom) decide which model handles each request.
- **App platform, not a single product.** Coding is one app among many.
- **P2P, not centralized.** Remote access and model sharing flow over WebRTC data channels. `linggen.dev` acts as a signaling relay; it does not see chat content.
- **Skills as the contract.** Apps follow the open Agent Skills standard.

---

## Remote access

```bash
ling login   # link to linggen.dev
```

Then open `linggen.dev/app` from any browser. P2P-encrypted tunnel back to
your machine; no VPN, no port forwarding.

---

## Use Linggen from other agents (MCP)

The daemon is an MCP server at `http://127.0.0.1:9898/mcp` — 22 tools in
four groups: `memory_*` (durable cross-host memory on the ling-mem store),
`browser_*` (drive one visible tab in your own Chrome, per-site permission
prompts), `x_*` (structured reads of your logged-in X session), and
`agent_run` (hand a task to a local Linggen agent and get its answer back).

On Claude Code / Codex, install the `linggen` plugin — it bundles the
per-turn memory recall hook and installs both binaries on first session
start:

```bash
claude plugin marketplace add linggen/linggen-memory
claude plugin install linggen@linggen-memory
```

Any other MCP client just points at the endpoint. Details in
[`doc/mcp-spec.md`](doc/mcp-spec.md).

---

## Documentation

- [Design docs](doc/) — architecture, specs, internals
- [Product spec](doc/product-spec.md) — system definition + design principles
- [Insight](doc/insight.md) — vision, roadmap, problems Linggen solves
- [Skill spec](doc/skill-spec.md) — how to write skills
- [Full docs](https://linggen.dev/docs) — guides and reference

---

## License

Apache 2.0 — engine and bundled skills. Branded apps shipped from [linggen-releases](https://github.com/linggen/linggen-releases) ship under their own terms.
