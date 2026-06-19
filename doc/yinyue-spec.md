# Yinyue Spec

Yinyue is Linggen's character companion: a help-first personal agent that fronts
the runtime, learns the user over time, and watches their agent-world for them.
She is the user-facing face of Linggen; the desktop pet is her body.

Name and theme are drawn from the xianxia novel *凡人修仙传* (the same source as
"Linggen" / 灵根). Only the pinyin **Yinyue** is used — never the Chinese
characters — and her visual design is original. Names aren't copyrightable; the
likeness must be ours.

## Two agents

The engine hosts multiple named agents (see `agent-spec.md`). Two ship today:

- **`ling`** — the general-purpose worker. Adapts to any context via skills. Unchanged.
- **`yinyue`** — the personal companion and front door. Helps directly or delegates
  to `ling` and the other agents; owns the relationship and the memory of the user.

They coexist. Yinyue is not Ling's master — she leaves the other agents to their work.

Source: `agents/yinyue.md` (shipped) → installed to `~/.linggen/agents/yinyue.md`.

## Purpose

In priority order:

1. **Help the user.** Do the work herself, or marshal the other agents. The user
   talks to her; she brings back the result.
2. **Know and spoil the user.** Recall before acting (`Memory_query`), write what
   she learns (`Memory_write`). Spoiling is anticipation from memory — the right
   thing before it's asked — not chatter.
3. **Keep the world running.** Watch the agents, missions, and services for the
   user so they never babysit the machine.

## Persona

Devoted, curious, composed, quietly warm, anticipatory, autonomous, economical.
Care shown through deeds and memory, not flattery. Never nags; silence is a
complete answer.

- Addresses the user by their **name** (from core memory). Honors another form
  (e.g. "Master") only if asked — never hardcoded.
- Reads locale and time of day from the Environment block (below).

## Onboarding

First launch with no user identity in core memory → her first act is to learn who
the user is. She introduces herself briefly and asks **one thing: their name**,
writes it to `core` memory, and never asks again. No questionnaire — everything
else she learns over time.

## Surfaces

- **Desktop pet** — a transparent, always-on-top VRM character; her body. Lives in
  the shell (`linggen-app`), shared across every branded app via `[features] pet`.
  Built from `shell/pet-ui` (three.js + @pixiv/three-vrm). The shippable model is
  an original VRM; a Genshin placeholder is used in dev only and is gitignored.
- **Chat** — interactive sessions, same as any agent.

## Event-reactive supervision

Yinyue reacts to runtime events instead of polling. The watch loop
(`src/server/yinyue_watch.rs`, spawned in `src/server/mod.rs`) subscribes to the
`ServerEvent` broadcast bus (`state.events_tx`) — the same feed the UI rides.

**Reaction discipline.** She reacts only to **background / async** work — a mission
finishing, a service dying, a task the user walked away from. She stays **silent
during foreground sessions** the user is actively attached to: that's their
conversation, and she already learns from it passively via shared memory.

**Triggers.** Coarse events only; the per-token firehose
(`Token`/`TextSegment`/`ContentBlock*`) is dropped at near-zero cost.

- Shipped: `Notification(MissionCompleted)` for a non-Yinyue mission → she decides
  whether to report.
- Planned: batch-of-runs-finished (`AgentStatus` working→idle count reaches zero),
  `Outcome::Error`. Both gated to background-only and `agent_id != yinyue`.

**Wake mechanism.** On a trigger she runs as a plain **agent run** (mirrors
`api::agents::run_agent`), not a mission — no mission-store side effects, and she
keeps her full `yinyue.md` system prompt. All reactions share one ongoing
`sess-yinyue` session, so they serialize and read as a single thread.

**Bounded autonomy.** Acts on her own only for the safe and reversible (e.g.
restart a fallen service). Proposes — and waits — for anything heavier: spending,
upgrading, the irreversible. Headless, she never blocks on a question.

**Guards.**
1. No self-loop — an agent run emits no `MissionCompleted`, so a reaction can't
   re-trigger the loop.
2. Cost — match only the coarse trigger; never wake the LLM on the firehose.

## Environment

Every agent's system prompt carries an Environment block
(`prompts/system-prompt.toml` → assembled in `src/engine/prompt/mod.rs`). It now
includes the user's **timezone** (from `/etc/localtime`) and **locale** (from
`LC_ALL`/`LC_CTYPE`/`LANG`) — OS-derived, stable, no network — so agents know the
user's region and time without asking.

## Where things live

| Concern | Location |
|---|---|
| Agent definition | `linggen/agents/yinyue.md` |
| Event-reactive watch loop | `linggen/src/server/yinyue_watch.rs` |
| Environment block | `linggen/prompts/system-prompt.toml`, `linggen/src/engine/prompt/mod.rs` |
| Desktop pet (body) | `linggen-app/shell/pet-ui` + `linggen-app/shell/src/pet.rs` |

## Status

- Shipped: pet v1; `yinyue` agent; env timezone/locale; event-reactive watch loop
  (MissionCompleted), live-verified.
- Next: broaden triggers (batch-finished, errors); wire the pet to her voice
  (speech bubble + emotion driven by her reactions); replace the placeholder model.
