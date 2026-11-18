<p align="center">
  <img src="logo.svg" width="120" alt="Linggen" />
</p>

<h1 align="center">Linggen</h1>

<p align="center">
  <strong>Your personal AI assistant, on your own Mac and iPhone.</strong><br>
  Ling and Yinyue look after a few parts of your life — music, money, health
  and play — and remember you across all of them. Local-first: it runs on
  your machine, with the model you choose.
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
  <img src="doc/media/demo.gif" width="800" alt="Linggen — CFO catches a double charge in locally-parsed statements, Apple Shifu scores your Mac's health, DJ runs karaoke with synced lyrics" />
</p>
<p align="center">
  <sub><strong>CFO</strong> catches a $389 double charge (statements never leave your Mac) · <strong>Apple Shifu</strong> scores your Mac's health · <strong>DJ</strong> runs karaoke night. <a href="https://linggen.dev">Full demo →</a></sub>
</p>

---

## Install

**The Mac app** — launcher with all apps below, menubar companion included:

```bash
curl -fsSL https://linggen.dev/install-app.sh | bash
```

**Engine only** — CLI + web UI, macOS and Linux (glibc 2.39+, Ubuntu 24.04 or newer):

```bash
curl -fsSL https://linggen.dev/install.sh | bash
ling
```

Opens the web UI at `http://localhost:9527`.

**Releases** — the Mac app (signed and notarized): [linggen-releases](https://github.com/linggen/linggen-releases/releases/latest) · the engine: [linggen/linggen](https://github.com/linggen/linggen/releases/latest) · what changed: [CHANGELOG.md](CHANGELOG.md)

**iPhone** — [Linggen on the App Store](https://apps.apple.com/app/id6798304906).

---

## What it looks after

Linggen is not a general chatbot and not an app store. It is one assistant
that takes a few parts of everyday life and does them well. Two share the
job: **Ling** does the work, **Yinyue** keeps you company and speaks up when
something matters — the same two on your Mac and your iPhone.

- **Music · DJ** — tell it a vibe and it curates a set, builds your local music library with clean tags, syncs tracks to your phone, and does karaoke with synced, translated lyrics.
- **Money · CFO** — a personal CFO that never uploads your money data. Drop in bank/card CSV or PDF exports: deterministic local code parses them into a spend report — monthly trends, subscriptions, commitments, transfer detection, duplicate-charge checks. The AI layer only explains and reviews, and account numbers are stripped before it sees even the redacted totals. [60-second demo](https://linggen.dev/cfo-demo.mp4).
- **Health** — your iPhone reads Apple Health and keeps your body data on the device. Ask what a week did to you or why a number moved, and get the answer in your own numbers. Wellness only.
- **Play · 灵境 (Lingjing)** — a cultivation story with ink-wash scenes, card duels and small games. Plays on the Mac and the iPhone; your save syncs through your account.
- **Play · Games** — Chinese Chess and Gomoku where the model actually plays you, plus Snake, Pong, and Tetris.
- **Your devices · Apple Shifu** — AI health analyst for your Mac: disk, security, performance, dormant apps, buyer's guide. Recommends; never acts on its own.

It remembers you: one memory on your machine, shared by Ling and Yinyue.

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

## For developers: how it is built

Skills, agents, missions — all files. New apps are a folder away. Browse community skills at [github.com/linggen/skills](https://github.com/linggen/skills).

- **Memory** — cross-agent semantic memory, part of Linggen: the same store reachable from Linggen, Claude Code, or Codex through the [`linggen` plugin](plugins/linggen).
- **Model sharing** — open a room and let friends use your models over P2P WebRTC. No keys for the consumer, no cloud middleman; the owner controls budget and tools.


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
| Filesystem | Memory store — LanceDB, hosted by the engine |
| Process privilege | Permission modes (chat / read / edit / admin) + path scoping |
| Network share | Rooms — share models with peers over P2P WebRTC |

Full table and design principles in [`doc/product-spec.md`](doc/product-spec.md);
vision and roadmap in [`doc/insight.md`](doc/insight.md).

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
- **Curated, not general.** One personal assistant for a few parts of life — music, money, health, play.
- **P2P, not centralized.** Remote access and model sharing flow over WebRTC data channels. `linggen.dev` acts as a signaling relay; it does not see chat content.
- **Skills as the contract.** Apps follow the open Agent Skills standard.

---

## Remote access

Sign in from the app (or `ling account login`). Signing in links this machine
automatically — there is no separate step.

Then open `linggen.dev/app` from any browser. P2P-encrypted tunnel back to
your machine; no VPN, no port forwarding.

---

## Hand work to Linggen from Claude Code or Codex (MCP)

The daemon is one MCP server at `http://127.0.0.1:9527/mcp`, with twelve
tools: memory (5), agents (4) and browser (3). Its `agent_run`
tool lets another agent hand a task to a local Linggen agent:

- **A different model does the work.** The Linggen agent runs on whatever you
  have set up here: a ChatGPT plan, a local model through Ollama, or an API
  key. The caller spends tokens on the brief and the answer, not on the file
  reads and test runs in between.
- **Writes stay in one folder.** The caller passes a git worktree as
  `write_dir`. Everything else on disk is read-only to the agent.
- **The caller answers permission requests.** A request for more goes back
  over a WebSocket to the agent that called; it answers with `agent_permit`
  (`allow_once`, `allow_session` or `deny`). No answer in ten minutes is a deny.
- **Long jobs run in the background.** `agent_run` returns a session id, and
  one frame arrives when the job ends. `agent_status` with `op: session` has the reply.

The story of using it day to day, with what the second agent got wrong:
[My Claude Code quota kept running out…](https://linggen.dev/blog/claude-code-hands-work-to-chatgpt-plan)

The same server carries the browser tools (`browser_open`, `browser_read`,
`browser_act`: drive one visible tab in your own Chrome, with per-site
permission prompts) and Linggen's durable memory (`memory_search`,
`memory_add`, `memory_update`, `memory_delete`, `memory_upkeep`). Another
computer you have approved sees the five memory tools only.

`agent_status` with `op: models` says which model can be used now and, for one
that cannot, why. A run that stops on a usage limit says so with its reset
time.

On Claude Code / Codex, install the `linggen` plugin (source:
[`plugins/linggen`](plugins/linggen)). It adds the one server and the per-turn
memory recall hook, and installs Linggen on first session start:

```bash
# Claude Code
claude plugin marketplace add linggen/linggen --sparse .claude-plugin plugins
claude plugin install linggen@linggen

# Codex
codex plugin marketplace add linggen/linggen
codex plugin add linggen@linggen
```

The browser tools need the Chrome extension, whose source is in this repo:
[`browser-extension/`](browser-extension).

Without git (a fresh Mac): `curl -fsSL https://linggen.dev/install-plugin.sh | bash`.

Then pick the model the Linggen agent runs on: in Linggen, Settings → Models →
**Sign in with ChatGPT**, or add an API key, or choose a local model.

Any other MCP client just points at the endpoint. Details in
[`doc/mcp-spec.md`](doc/mcp-spec.md).

---

## Documentation

- [Design docs](doc/) — architecture, specs, internals
- [Product spec](doc/product-spec.md) — system definition + design principles
- [Insight](doc/insight.md) — vision, roadmap, problems Linggen solves
- [Skill spec](doc/skill-spec.md) — how to write skills
- [Full docs](https://linggen.dev/docs) — guides and reference

## Credits

- Character animation credits to pixiv Inc.'s VRoid Project.

---

## License

Apache 2.0 — engine and bundled skills. Branded apps shipped from [linggen-releases](https://github.com/linggen/linggen-releases) ship under their own terms.
