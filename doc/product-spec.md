---
type: spec
guide: |
  Product specification — describe what the system should do and why.
  Keep it brief. Aim to guide design and implementation, not document code.
  Avoid implementation details like function signatures, variable types, or code snippets.
---

# Product Spec: Linggen

## Vision

Linggen is **the root system for AI agents**. The core runtime manages agent processes, communication, and execution — everything else grows on top as skills and agents.

Users manage agents, skills, and models through Web UI and TUI. Each agent has its own context, skill set, and mission — from a social chat skill to a coding agent to a scheduled architecture guardian. Skills follow the [Agent Skills](https://agentskills.io) open standard (aligned with Claude Code) and work across AI tools.

For vision, landscape, and roadmap, see [`insight.md`](insight.md).

### OS analogy

| OS Concept | Linggen Equivalent |
|:-----------|:-------------------|
| Process | Agentic loop — one running agent |
| Interrupt | User message queue — checked each iteration, model decides |
| Thread/Fork | Subagent delegation — concurrent child execution |
| Syscall | Tool call — built-in tools are the kernel API |
| Dynamic library | Skill — loaded at runtime, no code changes |
| Shell/scripting | Bash tool — model runs shell commands |
| Signal | Cancel, pause — delivered via message queue |
| IPC | Event bus + delegation for inter-agent communication |
| Cron job | Mission — cron-scheduled agent task (multiple per project) |
| Driver | Model provider — Ollama, OpenAI, Claude, Bedrock |

### Related docs

| Doc | OS Layer |
|:----|:---------|
| `agentic-loop.md` | Kernel — loop, interrupts, cancellation |
| `agents.md` | Process management — lifecycle, delegation, scheduling |
| `skills.md` | Dynamic extensions — format, discovery, triggers |
| `tools.md` | Syscall interface — built-in tools, safety |
| `chat-spec.md` | Chat system — SSE events, message model, rendering, APIs |
| `models.md` | Hardware abstraction — providers, routing |
| `cli.md` | Shell — CLI reference |
| `insight.md` | Vision, landscape, roadmap |

---

## Design Principles

### 1. Skills-first architecture

Skills are the primary extension mechanism. Built-in tools only for core operations needing safety/state/performance. Everything else (web search, APIs, git workflows) is a skill.

- Each skill is a directory with `SKILL.md` entrypoint and optional supporting files.
- Adding a skill = dropping a directory. No code changes needed.
- Skill format aligns with Claude Code and the Agent Skills open standard.

### 2. Multi-agent management

- Framework ships with default agents; users create more by dropping `agents/*.md` files.
- Each agent has its own context, skill set, model preference, and policy.
- Agents range from general-purpose to specialized (coding, social chat, research).
- Switch via `/agent <name>` or tab views in web UI.

### 3. Unified CLI and shared sessions

- Single command `linggen` starts backend server + TUI.
- Web UI and TUI share session state via same HTTP/SSE API.
- Chat messages from any client appear in real-time on all clients.

### 4. Trigger symbols

Parsed from user input only (not model output):

- `/` — built-in commands and skill invocation.
- `@` — mentions, routed to skills for the named target.
- User-defined triggers via skill frontmatter.

### 5. Multi-model routing

Users configure multiple providers (Ollama, OpenAI, Claude, Bedrock). Named routing policies (`local-first`, `cloud-first`, custom) control which model handles each request. See `models.md`.

### 6. Cross-tool skill ecosystem

- Skills written for Linggen work in Claude Code and Codex (shared Agent Skills standard).
- Users manage all agents, skills, and models from one place.

---

## Product Goals

- **Root system** — robust core runtime (agentic loop, message queue, agent lifecycle, event bus).
- **Skills-first** — add capabilities by dropping a `SKILL.md`. Built-in tools only for core ops.
- **User interrupt** — users can message a running agent; model sees it and adapts.
- **Multi-agent concurrency** — multiple agents running simultaneously.
- **Unified CLI** — serves both TUI and web UI from shared backend.
- **Multi-model routing** — named policies (local-first, cloud-first, custom).
- **Cross-tool compatibility** — Agent Skills standard.

## Mission System

Cron-based scheduled agent work — like a crontab with multiple entries.

- **No missions** → agents are purely reactive (human-in-the-loop).
- **Active missions** → agents triggered on cron schedules, each with its own prompt.

A project can have **multiple active missions**. Each mission defines a cron schedule, target agent, prompt, and optional model override. Missions are independent — enable, disable, or edit them individually. See [`mission-spec.md`](doc/mission-spec.md) for full details.

## UX Surface

- **CLI**: `ling` starts backend server + TUI.
- **Web UI**: agent/skill management, session chat, agent tab views, mission page, settings, memory.
- **Agent switching**: `/agent <name>` or tab views.
- **Remote access**: power users proxy the Web UI remotely (SSH tunnel, reverse proxy). Future: built-in secure proxy or WebRTC.

## Safety Requirements

- Workspace-scoped file operations.
- Allowlisted command execution.
- Persisted chat/run records.
- Cancellation cascades through run trees.
- See `tools.md` for details.

## Non-goals (early stage)

- Multi-tenant hosted SaaS.
- Unbounded autonomous production deployment.
- Container/VM sandboxing (local trust model).
