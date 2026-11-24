---
type: spec
reader: Coding agent and users
guide: |
  Product specification — describe what the system should do and why.
  Keep it brief. Aim to guide design and implementation, not document code.
  Avoid implementation details like function signatures, variable types, or code snippets.
---

# Product Spec: Linggen

## Vision

Linggen is **a local AI app engine — and your general-purpose personal assistant**. Two faces of the same runtime: out of the box, the assistant chats and acts; install skills and the same runtime hosts them as full apps.

Architecturally, Linggen is **the root system for AI agents** — the core runtime manages agent processes, communication, and execution; everything else (skills, agents, missions) grows on top as files. Skills follow the [Agent Skills](https://agentskills.io) open standard (aligned with Claude Code) and work across AI tools.

### The kernel boundary

**No skill-specific Rust in the engine.** The engine is a general core. Every capability it offers must be usable by any skill and named in general terms — never after one app. A skill is external: markdown, HTML, JS, and its own scripts.

When a skill needs something the engine can't do, the fix is a **general primitive the skill declares or calls**, never a module named after the skill. If it can't be generalized, it doesn't belong in the engine yet.

Test before adding engine code: *would a second, unrelated skill use this?* If no, it's the skill's job.

For background on why an AI app engine is needed, plus landscape and roadmap, see [`insight.md`](insight.md).

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
| Cron job | Mission — scheduled agent / app / script (multiple per project) |
| Driver | Model provider — Ollama, OpenAI, Claude, Bedrock |
| Filesystem | Memory store — core markdown + LanceDB RAG via `ling-mem` |
| Process privilege | Permission modes (chat / read / edit / admin) + path scoping |
| User account | `UserContext` — every peer has `user_id`; state filtered per-user |
| Network share | Rooms — share models with peers over P2P WebRTC |

### Related docs

| Doc | OS Layer |
|:----|:---------|
| `agentic-loop.md` | Kernel — loop, interrupts, cancellation |
| `agent-spec.md` | Process management — lifecycle, delegation, scheduling |
| `session-spec.md` | Session/context — creators, effective tools, prompt assembly |
| `skill-spec.md` | Dynamic extensions — format, discovery, triggers, install hooks |
| `mission-spec.md` | Cron jobs — agent / app / script, scope, scheduling |
| `tool-spec.md` | Syscall interface — built-in tools, safety |
| `permission-spec.md` | Privilege model — modes, layers, deny floor, remote trust |
| `chat-spec.md` | Chat system — events, message model, rendering, APIs |
| `plan-spec.md` | Plan mode — research-then-execute |
| `memory-spec.md` | Memory — extraction, storage, two-tier loading |
| `models.md` | Hardware abstraction — providers, routing |
| `webrtc-spec.md` | Transport — P2P, signaling, data channels |
| `room-spec.md` | Sharing — rooms, shared models, token budgets |
| `storage-spec.md` | Filesystem layout — `~/.linggen/` |
| `log-spec.md` | Logging levels, throttling, output targets |
| `cli.md` | Shell — CLI reference |
| `insight.md` | Vision, landscape, roadmap |

---

## Settings

The browser and the Mac app open the same full-window settings page. In the Mac app, Back or Esc returns to the selected app without restarting it. Desktop controls appear only inside the native shell.

## Design Principles

### 0. Design for five years from now — build what stronger models will not take away

Hanli, 2026-11-24: 「设计要面向未来5年. 5年后llm会强大到, agent可以做得很薄, 只给简单工具就能让llm完成任务.
因此linggen的设计要能提供差异化, focus在不会被吃掉的部分, 比如用户的情绪价值, yinyue对llm不做的部分的,
面向人类的设计, 情感第一, 人情世故第二.」

Assume that in five years a model with a handful of simple tools does what today needs a thick harness. Whatever a
stronger model, or the model makers' own free agents, will do by themselves is not where Linggen spends its effort.

**Invest in what stays human:**
1. **Feeling first** — the emotional value of a companion: Yinyue's warmth, continuity, presence (voice, face,
   phone), being remembered as a person (body, mind, family, small moments), play that people come back to.
2. **人情世故 second** — social sense: when to speak and when to stay quiet, tact, timing, care for the people around
   the person, the things a good friend remembers without being asked.
3. **What only the person's own devices hold** — their life data and senses (health, music, money, play), kept on
   their devices, and the trust that comes with that.

**Keep thin, do not race:** agent loops, tool orchestration, prompt tricks, coding-agent features, task delegation,
retrieval plumbing, model routing. Keep them general and small (Hanli 2026-11-15: 「工具要底层通用, 数量尽量少, 尽量让agent去做」 — few
low-level tools, the model judges), take the vendors' improvements, and do not build a moat there.

**The goal this serves — a personal companion, people first** (Hanli, 2026-11-24): 「linggen不仅是用户的情感伴侣,
还是人生导师… yinyue要像人, linggen要以人为本.」 Linggen grows up with the person over years and, like a good friend
and mentor, connects what it has seen: two years after a break-up the person has thrown themselves into research and
done well — now and then she may say it is also time to think of their own life; last Black Friday they regretted a
1,000 spend — this Black Friday she reminds them. These are examples, not features: Yinyue gets the person's long
record (memory and chronicle) and the date, and judges for herself when and how to speak (Hanli 2026-11-15: no tool or
coded condition per scenario).

**The test for every design:** if models were ten times stronger, would this still matter, and would it still make
Linggen different? If not, build the thinnest version or none.

### 1. Skills-first architecture

Skills are the primary extension mechanism. Built-in tools only for core operations needing safety/state/performance. Everything else (web search, APIs, git workflows) is a skill.

- Each skill is a directory with `SKILL.md` entrypoint and optional supporting files.
- Adding a skill = dropping a directory. No code changes needed.
- Skill format aligns with Claude Code and the Agent Skills open standard.

### 2. Multi-agent management

- Framework ships with default agents; users create more by dropping `agents/*.md` files.
- Each session has its own context, effective tools, model, and optional bound skill.
- One default agent: `ling` (versatile, adapts to any context via skills). Missions are cron-scheduled tasks, not a separate agent.
- Switch via `/agent <name>` or tab views in web UI.

### 3. Unified CLI and shared sessions

- Single command `ling` starts backend server + opens Web UI.
- All communication over WebRTC data channels.
- Chat messages from any client appear in real-time on all clients.

### 4. Trigger symbols

Parsed from user input only (not model output):

- `/` — built-in commands and skill invocation.
- `@` — mentions, routed to skills for the named target.
- User-defined triggers via skill frontmatter.

### 5. Multi-model routing

Users configure multiple providers (Ollama, OpenAI, Claude, Bedrock). An ordered list of default models picks which model handles each request, with auto-fallback on transient errors. See `models.md`.

### 6. Cross-tool skill ecosystem

- Skills written for Linggen work in Claude Code and Codex (shared Agent Skills standard).
- Users manage all agents, skills, and models from one place.

### 7. Durable memory

The agent remembers who the user is and how to work with them across sessions. Shipped as a skill (`ling-mem`) so the same store is reachable from any AI tool. See `memory-spec.md`.

### 8. Sharing without the cloud

Owners can open rooms to share their models with others. Inference flows P2P over WebRTC — no cloud middleman. See `room-spec.md`.

---

## Product Goals

- **Root system** — robust core runtime (agentic loop, message queue, agent lifecycle, event bus).
- **Skills-first** — add capabilities by dropping a `SKILL.md`. Built-in tools only for core ops.
- **User interrupt** — users can message a running agent; model sees it and adapts.
- **Multi-agent concurrency** — multiple agents running simultaneously.
- **Unified CLI** — `ling` starts the server and opens the Web UI.
- **Multi-model routing** — ordered default models with auto-fallback.
- **Cross-tool compatibility** — Agent Skills standard.
- **Durable memory** — persistent identity, preferences, and trajectory across sessions.
- **Own-your-models sharing** — proxy rooms over P2P WebRTC, no cloud middleman.

## Mission System

Schedule-driven work — a crontab with multiple entries. Each mission runs as an agent, an app launch, or a shell script, with its own permission scope and tool/skill allowlist. See [`mission-spec.md`](mission-spec.md).

## Rooms

Owners open private or public rooms to share their models with others. Inference flows over the same P2P WebRTC link as remote access — the relay never sees request bodies. Per-room and per-consumer daily token budgets. See [`room-spec.md`](room-spec.md).

## UX Surface

- **CLI**: `ling` starts backend server + opens Web UI.
- **Web UI**: agent/skill management, session chat, missions, room sharing, settings, memory.
- **Remote access**: P2P WebRTC from anywhere via `linggen.dev`. See `webrtc-spec.md`.
- **Sharing**: open a room; friends connect from `linggen.dev/app`. See `room-spec.md`.

## Safety Requirements

- Workspace-scoped file operations.
- Permission modes (chat / read / edit / admin) with path scoping and a hardcoded deny floor. See `permission-spec.md`.
- Per-user filtering on remote peers.
- Persisted chat/run records; cancellation cascades through run trees.

## Non-goals (early stage)

- Multi-tenant hosted SaaS.
- Unbounded autonomous production deployment.
- Container/VM sandboxing (local trust model).
