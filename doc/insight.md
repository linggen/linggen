# Vision & Roadmap

> Last updated: 2026-04-29
>
> For system definition, OS analogy, and design principles, see [`product-spec.md`](product-spec.md).

---

## Background

Models are getting smarter, and they have broken the old way we engineer an app. The LLM replaces the if-else logic in code: it can handle real-world problem complexity that isn't continuous — the kind of branching that resists being written down ahead of time. The model will be the brain of an app.

But the model alone is not enough. Until there is a real working world model that lets it fully understand circumstance, it still struggles to build apps on its own. And on the other side, non-technical users struggle even more: they don't know how a server and a UI work, what widget to use, how the protocol part should be done. The promise of "anyone can build with AI" runs into the reality that you still need a substrate around the model.

That gap is where Linggen lives. By harnessing the model and exposing a generic toolkit — `pageUpdate` for UI, a memory system for state, permissions, scheduling, P2P sharing — Linggen lets the agent drive the building. The model becomes aware of the workflow, controls the app's logic, and renders its UI. The non-tech user describes what they want; the agent assembles a working app on top of the runtime. The result is twofold: normal users can build apps without knowing software, and the apps themselves are smarter than the old fashioned ones because the brain is in the loop at runtime, not frozen at ship time.

## What Linggen Is

Linggen is **a local AI app engine — and your general-purpose personal assistant**. Two faces of the same runtime: out of the box, the assistant chats and acts; install skills and the same runtime hosts them as full apps.

An "AI app" in Linggen is a skill, an agent, or a mission — markdown + scripts, not code plugins. The runtime gives every app a process (agent loop), syscalls (built-in tools), a filesystem (memory), permissions, and a network surface (P2P rooms). Apps drop into a folder and run.

Architecturally Linggen is **the root system for AI agents** — agents are processes, skills are dynamic libraries, missions are cron jobs. See [`product-spec.md`](product-spec.md) for the full OS analogy and design principles.

---

## Apps Built on Linggen

**Mac Shifu** — `mac-shifu` skill. AI-driven system health analyst with its own web dashboard. Scans disk, apps, caches; suggests cleanup commands. An AI app, not a chatbot.

**Memory** — `ling-mem` skill. LanceDB-backed semantic memory: typed facts, embeddings, filter-and-search, first-class forgetting. Same store reachable from Linggen, Claude Code, or any tool that can shell out.

**Model Sharing** — Rooms. Open one and let friends use your models over P2P WebRTC. No keys for the consumer, no cloud middleman, owner controls budget and tools.

**Architecture Guardian** — Agent + mission. Reviews code and updates dependency graphs on a schedule, flags design violations.

**DevOps** — Mission. Monitor CI/CD, auto-fix flaky tests, manage deployments — defined in markdown.

Skills, agents, missions — all files. New apps are a folder away.

---

## Where Linggen Sits

- **Local-first.** The runtime, the data, and inference (when local models are picked) all live on the user's machine. Cloud is opt-in and goes through user-owned API keys.
- **Model-agnostic.** Any model — local Ollama, Claude, GPT, Gemini, OpenRouter — and routing policies decide which one handles each request.
- **App platform, not a single product.** The agent loop, tool surface, permission system, memory, and P2P fabric are general-purpose. Coding is one app among many.
- **P2P, not centralized.** Remote access and model sharing flow over WebRTC data channels. linggen.dev acts as a signaling relay and account directory; it does not see chat content.
- **Skills as the contract.** Apps follow the open [Agent Skills](https://agentskills.io) standard, so the same skill works in Linggen, Claude Code, and Codex.

---

## Problems We Solve

| Problem | Current State | Linggen's Answer |
| --- | --- | --- |
| AI tools are single-purpose | Each tool does one thing (IDE, terminal, chat) | One runtime hosts diverse apps — coding, diagnostics, memory, social |
| Extending AI requires code | MCP servers, plugins, SDKs | Drop a folder (markdown + scripts) |
| AI is single-shot | One conversation, no recurring tasks | Missions = real cron jobs (agent / app / script modes) |
| AI forgets between sessions | Re-explain context every time | Semantic memory store (`ling-mem`) shared across tools |
| Everyone needs their own API keys | Cold-start friction kills adoption | Rooms — owners share spare capacity P2P, no cloud middleman |
| Privacy concerns | Most tools send code to cloud | Local-first, model-agnostic, user-controlled |
| Unpredictable costs | $5-15/session, credit exhaustion | Route to local models for routine tasks |
| "Agent OS" is enterprise-only | Industrial workflow platforms target enterprise | Open app engine for individuals and developers |

---

## Roadmap

### Focus areas

- **Core runtime** — scheduling, interruption, multi-agent coordination, tool execution, safety
- **Apps as files** — zero-code-change extensibility via markdown skills, agents, and missions
- **Open standards** — MCP, Agent Skills, AGENTS.md
- **Model-agnostic routing** — connect any model, route intelligently
- **P2P fabric** — remote access and model sharing without cloud middlemen
- **Skills marketplace** — community-driven app ecosystem

### Non-goals

- Competing on model intelligence — that's the providers' job
- Enterprise multi-tenant orchestration — Linggen is personal-first
- Replacing the IDE — code-editing apps live on top of the runtime, not under it
- Container/VM sandboxing — the runtime trusts the host machine

### Shipped (highlights since v0.8)

- **Remote access over WebRTC** — P2P data channels, signaling relay on linggen.dev, UI loaded from server via data channel.
- **Rooms** — owners share their models with consumers over P2P; per-room and per-consumer daily token budgets; private + public rooms.
- **User isolation** — every peer carries a `user_id`; sessions, models, skills, and busy state are filtered per-user before delivery.
- **Semantic memory** — `ling-mem` skill, LanceDB store, typed facts, cross-tool.
- **Mission as first-class subsystem** — `agent` / `app` / `script` modes with per-mission permission and tool/skill scope.
- **linggen.dev SaaS** — auth (GitHub + Google), dashboard, signaling relay, usage tracking, bill guard.

### Planned

- **Rooms — credits + auto-dispatch.** Earn-by-proxying, spend-by-consuming, starter credits for new users; dispatcher matches consumers to the best available room.
- **Pricing & billing.** Pay-as-you-go on linggen.dev ($0.10/day, $10 cap); Stripe integration.
- **Skills marketplace deeper** — ratings, install counts surfaced in UI, discovery beyond text search.
- **More first-party apps** — keep proving the runtime by shipping useful apps (memory, diagnostics, sharing) before broader audiences.
