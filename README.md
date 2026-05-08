<p align="center">
  <img src="logo.svg" width="120" alt="Linggen" />
</p>

<h1 align="center">Linggen</h1>

<p align="center">
  <strong>A local AI app engine — and your general-purpose personal assistant.</strong><br>
  Two faces of one runtime: out of the box, the assistant chats and acts;
  install skills and the same runtime hosts them as full apps.
</p>

<p align="center">
  <a href="https://linggen.dev">Website</a> &middot;
  <a href="https://linggen.dev/apps/sysdoctor">Apps</a> &middot;
  <a href="https://linggen.dev/skills">Skills</a> &middot;
  <a href="https://linggen.dev/docs">Docs</a> &middot;
  <a href="https://discord.gg/linggen">Discord</a>
</p>

<p align="center">
  <a href="https://github.com/linggen/linggen/releases"><img src="https://img.shields.io/github/v/release/linggen/linggen?style=flat-square" alt="Release" /></a>
  <a href="https://github.com/linggen/linggen/blob/main/LICENSE"><img src="https://img.shields.io/github/license/linggen/linggen?style=flat-square" alt="Apache 2.0 License" /></a>
  <a href="https://github.com/linggen/linggen/stargazers"><img src="https://img.shields.io/github/stars/linggen/linggen?style=flat-square" alt="Stars" /></a>
</p>

---

## Install

```bash
curl -fsSL https://linggen.dev/install.sh | bash
ling
```

Opens the web UI at `http://localhost:9898`. macOS and Linux.

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

## Apps built on Linggen

- **[Sys Doctor](https://linggen.dev/apps/sysdoctor)** — AI health analyst for your Mac. Disk, security, performance, dormant apps, buyer's guide. Bundled `.app` available.
- **Memory** — `ling-mem` skill. LanceDB semantic store with typed facts, embeddings, first-class forgetting. Same store reachable from Linggen, Claude Code, or any tool that can shell out.
- **Model Sharing** — Rooms. Open one and let friends use your models over P2P WebRTC. No keys for the consumer, no cloud middleman, owner controls budget and tools.
- **Architecture Guardian** — Agent + mission. Reviews code and updates dependency graphs on a schedule, flags design violations.
- **DevOps** — Mission. Monitor CI/CD, auto-fix flaky tests, manage deployments — all defined in markdown.

Skills, agents, missions — all files. New apps are a folder away. Browse community skills at [linggen.dev/skills](https://linggen.dev/skills).

---

## Add an app

Drop a markdown file in `~/.linggen/` — available immediately, no restart:

```markdown
---
# ~/.linggen/agents/reviewer.md
name: reviewer
description: Code review specialist.
tools: ["Read", "Glob", "Grep"]
model: claude-sonnet-4-20250514
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

## Documentation

- [Design docs](doc/) — architecture, specs, internals
- [Product spec](doc/product-spec.md) — system definition + design principles
- [Insight](doc/insight.md) — vision, roadmap, problems Linggen solves
- [Skill spec](doc/skill-spec.md) — how to write skills
- [Full docs](https://linggen.dev/docs) — guides and reference

---

## Telemetry

Linggen sends a small amount of anonymous usage data to `https://linggen.dev/api/track` so we can see whether the project is being used and where to invest. Specifically:

- **`install`** — once on first launch on a machine, and once after each upgrade. Includes the install source (`wrapper`, `brew`, `cargo`, `sys-doctor`, `unknown`) and the previous + current versions.
- **`command`** — one event per meaningful action: `engine.start` (each daemon start), `session.start`, `skill.<name>.open`, etc. The verb is a short stable identifier; no chat content, no file paths, no model output.
- **`system_state`** — included in `engine.start` payload: which sibling Linggen products (Sys Doctor, ling-mem) are detected on this machine via marker files in `~/.linggen/`. Lets us track adoption of those products without each needing its own phone-home.

What's **never** sent: chat messages, prompts, model responses, file contents, paths, embeddings, your IP (the receiver doesn't store it), or any user-identifying string. The `installation_id` is a random UUIDv4 generated on first run and stored at `~/.linggen/installation_id`.

**Disabling telemetry:**

- Runtime: set `LINGGEN_NO_TELEMETRY=1`, or `touch ~/.linggen/no-telemetry`.
- Compile time: build with `cargo build --release --no-default-features` (no telemetry code is even linked in).

Source is open on both ends: client at [`src/telemetry/`](src/telemetry/), receiver at [`linggensite/functions/api/_lib/analytics.ts`](https://github.com/linggen/linggensite/blob/main/functions/api/_lib/analytics.ts). No third-party analytics — only `linggen.dev/api/track`.

---

## License

Apache 2.0 — engine and bundled skills. Branded apps shipped from [linggen-releases](https://github.com/linggen/linggen-releases) ship under their own terms.
