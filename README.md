<p align="center">
  <img src="logo.svg" width="120" alt="Linggen" />
</p>

<h1 align="center">Linggen</h1>

<p align="center">
  <strong>Local AI coding agent you can access from anywhere.</strong><br>
  Open-source. Any model. WebRTC remote access. Skills you can share.
</p>

<p align="center">
  <a href="https://linggen.dev">Website</a> &middot;
  <a href="https://linggen.dev">Demo Video</a> &middot;
  <a href="https://linggen.dev/docs">Docs</a> &middot;
  <a href="https://linggen.dev/skills">Skills Marketplace</a> &middot;
  <a href="https://discord.gg/linggen">Discord</a>
</p>

<p align="center">
  <a href="https://github.com/linggen/linggen/releases"><img src="https://img.shields.io/github/v/release/linggen/linggen?style=flat-square" alt="Release" /></a>
  <a href="https://github.com/linggen/linggen/blob/main/LICENSE"><img src="https://img.shields.io/github/license/linggen/linggen?style=flat-square" alt="MIT License" /></a>
  <a href="https://github.com/linggen/linggen/stargazers"><img src="https://img.shields.io/github/stars/linggen/linggen?style=flat-square" alt="Stars" /></a>
</p>

---

## What is Linggen?

Linggen is an AI coding agent that runs on your machine — and lets you access it from any device via WebRTC. Start a task on your desktop, check on it from your phone. No cloud hosting, no subscriptions, your models and your data.

```bash
curl -fsSL https://linggen.dev/install.sh | bash
ling
```

That's it. Opens a web UI at `localhost:9898`.

## Why Linggen over Claude Code / Cursor / Codex?

| | Linggen | Claude Code | Cursor | Codex |
|---|---|---|---|---|
| **Runs locally** | Yes | Yes | No (cloud) | Cloud-only |
| **Any model** | Ollama, Claude, GPT, Gemini, DeepSeek, Groq, OpenRouter | Claude only | Multi-model | GPT only |
| **Remote access** | Built-in WebRTC — use from any device | No | No | No |
| **Open source** | MIT | No | No | CLI only |
| **Skills/extensions** | Drop-in SKILL.md files ([Agent Skills](https://agentskills.io) standard) | Custom slash commands | Plugins | No |
| **Web UI** | Full web interface with streaming | Terminal only | IDE-embedded | Web (cloud) |
| **Cost** | Free + your model costs | $20/mo or API costs | $20/mo | API costs |

## Key Features

### Remote Access via WebRTC

Start a coding task on your desktop, monitor it from your phone. No VPN, no port forwarding — peer-to-peer encrypted connection.

```bash
ling login   # link to linggen.dev
```

Then open `linggen.dev/app` from any browser, anywhere.

### Any Model, Your Choice

Use local models via Ollama, or cloud APIs — Claude, GPT, Gemini, DeepSeek, Groq, OpenRouter. Switch models mid-conversation. Configure fallback chains so work never stops.

### Skills, Not Plugins

Drop a `SKILL.md` into your project and the agent gains new capabilities instantly. Skills follow the open [Agent Skills](https://agentskills.io) standard, compatible with Claude Code and Codex.

```
~/.linggen/skills/my-skill/SKILL.md
```

Browse and install community skills from the [marketplace](https://linggen.dev/skills).

### Multi-Agent Delegation

Agents delegate tasks to other agents — each with its own context, tools, and model. Like `fork()` for AI.

### Plan Mode

For complex tasks, the agent proposes a plan before acting. Review, edit, or approve — then it executes. Stay in control on high-stakes changes.

### Mission System

Schedule recurring tasks with cron expressions. Code reviews, dependency updates, monitoring — agents self-initiate work on your schedule.

## Quick Start

```bash
# Install
curl -fsSL https://linggen.dev/install.sh | bash

# First-time setup
ling init

# Start (opens browser)
ling

# Optional: enable remote access
ling login
```

## Adding Agents

Drop a markdown file in `~/.linggen/agents/` — available immediately, no restart:

```markdown
---
name: reviewer
description: Code review specialist.
tools: ["Read", "Glob", "Grep"]
model: claude-sonnet-4-20250514
---

You review code for bugs, style issues, and security vulnerabilities.
```

## Documentation

- [Design docs](doc/) — architecture, specs, and internals
- [Full docs](https://linggen.dev/docs) — guides and reference
- [Skill spec](doc/skill-spec.md) — how to write skills

## Contributing

Contributions welcome. See the [design docs](doc/) for architecture context.

## License

MIT
