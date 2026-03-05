# Vision & Roadmap

> Last updated: 2026-03-05
>
> For system definition, OS analogy, and design principles, see [`product-spec.md`](product-spec.md).

---

## What Linggen Is

Linggen is a **Personal AI OS** — the runtime on your machine that runs AI agents as apps. Agents, skills, and missions are files (markdown + scripts), not code plugins. Multiple agents run concurrently with multi-model routing, delegation, and cooperative interruption. See `product-spec.md` for the full system design.

---

## Example Use Cases

**Disk Cleanup** — A skill + mission that analyzes disk usage weekly, suggests cleanup, asks for confirmation before deleting.

**Discord Social** — The `discord` skill lets users chat with friends via `@@friend_name message`. The agent becomes a social interface, not a coding tool.

**Architecture Guardian** — `architect.md` agent with a mission: review code and update Mermaid dependency graphs every 30 minutes, flag design violations.

**Memory & Learning** — An agent that watches your workflow, stores decisions in filesystem memory, surfaces context when you revisit old projects.

**DevOps** — Monitor CI/CD, auto-fix flaky tests, manage deployments — all defined in markdown, running as a mission.

---

## Landscape (March 2026)

### AI Coding Assistants

| Player | Scale | Positioning |
|---|---|---|
| Cursor | $2B+ ARR | AI-native IDE, dominant daily driver |
| GitHub Copilot | Largest install base | Universal baseline, $10-39/mo, multi-model |
| Claude Code | Growing fast | Terminal agent for complex tasks, 1M token context |
| OpenAI Codex | 1M+ devs | Cloud-sandboxed parallel agents |
| Aider | OSS gold standard | Git-native CLI, best efficiency metrics |
| Cline | 5M+ devs | Governance-first VS Code agent |

### AI Personal Assistants

| Player | Scale | Positioning |
|---|---|---|
| OpenClaw | 264K GitHub stars | OSS personal AI on messaging apps (WhatsApp, Telegram, 20+) |
| Apple Intelligence | Platform-embedded | On-device models, routes to ChatGPT for hard tasks |
| Google Gemini | Platform-embedded | 1M context, Deep Think, custom Gems, MCP support |
| Microsoft Copilot | Platform-embedded | GPT-5, connectors to Office/Google, local LLM on Windows |

### Agent Orchestration

| Player | Status | Positioning |
|---|---|---|
| LangGraph | Industry standard | Production-grade stateful multi-agent (enterprise) |
| CrewAI | Strong adoption | Role-based agents, lowest barrier to entry |
| Microsoft Agent Framework | RC | Replaces AutoGen + Semantic Kernel |

### "Agent OS" Players

AIOS, ElizaOS, Palantir AIP, Siemens-NVIDIA Industrial AI OS — all target enterprise workflows, not personal/developer use.

---

## Where Linggen Sits

Linggen is **none of these categories**:
- Not a coding assistant (agents can code, but also clean disks, chat on Discord, guard architecture)
- Not an enterprise orchestration platform (personal, local-first)
- Not a messaging hub (rich Web UI, not chat-app text tubes)
- Not a model provider (routes to any model)

**Linggen is the personal AI OS** — combining multi-agent orchestration, file-based extensibility, cron missions, and a rich UI for personal/developer use.

### Linggen vs OpenClaw

OpenClaw (264K stars) looks similar on the surface — local-first, personal, extensible, model-agnostic. But fundamentally different:

| | **OpenClaw** | **Linggen** |
|---|---|---|
| Core metaphor | AI in your messaging apps | AI OS on your workstation |
| Interface | WhatsApp, Telegram, Discord (20+ channels) | Web UI, TUI, CLI, VS Code |
| Agent model | Single agent (Pi) | Multi-agent with delegation |
| Autonomy | Cron jobs, webhooks | Missions (cron-scheduled agent tasks) |
| Extensibility | Skills (ClawHub registry) | Skills + Agents + Missions (all files) |
| Optimizes for | **Reach** (20+ chat channels) | **Richness** (full OS interface) |

---

## The Problems We Solve

| Problem | Current State | Linggen's Answer |
|---|---|---|
| AI tools are single-purpose | Each tool does one thing (IDE, terminal, chat) | One OS runs diverse agents (coding, social, devops, cleanup) |
| Extending AI requires code | MCP servers, plugins, SDKs | Drop a folder (markdown + scripts) |
| AI is single-shot | One conversation, no recurring tasks | Missions = real cron jobs for AI agents |
| AI doesn't understand your system | Architecture-blind code generation | Agents with filesystem memory + missions |
| Privacy concerns | Most tools send code to cloud | Local-first, model-agnostic, user-controlled |
| Unpredictable costs | $5-15/session, credit exhaustion | Route to local models for routine tasks |
| "Agent OS" is enterprise-only | AIOS, Palantir, etc. target workflows | Personal AI OS for developers and power users |

---

## Roadmap

### Focus areas

- **OS kernel** — scheduling, interruption, multi-agent coordination, tool execution, safety
- **Apps as files** — zero-code-change extensibility via markdown agents, skills, and missions
- **Open standards** — MCP, Agent Skills, AGENTS.md
- **Local-first** — privacy + cost control as a core differentiator
- **Skills marketplace** — community-driven skill ecosystem
- **Real "apps"** — disk cleanup, Discord, architect guardian — show the OS is general-purpose

### Non-goals

- Competing on model intelligence (that's the providers' job)
- Enterprise orchestration (LangGraph/CrewAI territory)
- Being an IDE (Cursor/Zed territory)
- Chat-app integration (OpenClaw's path — reach over richness)

### Planned

- `linggen-agent` rename to `linggen` (the agent IS the product)
- Built-in secure remote access (WebRTC or proxy — no SSH tunnels needed)
- Skills marketplace with community contributions
