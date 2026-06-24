# Yinyue — the living companion

Yinyue is the user's spirit companion and the face on the desktop. This spec
covers making her *proactive* and *interactive*: she reacts to what's happening,
reads whether the user is present, speaks and emotes in her own voice, and relays
between the user and the worker agents.

Ling and Yinyue are **separate agents** (extensible to more). Yinyue is the
herald / front-of-house voice for the whole roster; Ling and future workers do
the technical work.

## The one spine

Every proactive surface funnels through one rule:

> **Yinyue observes** (an event, a scheduled glance, a relayed user input)
> **→ reads her senses** (presence + state)
> **→ decides** to speak / emote / relay / stay silent
> **→ acts** via `PetSpeak` / `Express` / `answer_prompt`.

She **always phrases her own lines** — never a hardcoded/templated string. The
trigger decides she *takes a look*; **presence governs the response**.

Inputs converge on her existing watch loop (`server/yinyue_watch.rs`) and
`run_yinyue_turn`:

- **events** (system) — terminal/blocked run signals on the broadcast bus
- **agent_chat** (peer) — a message addressed to her from another agent
- **timer** (ambient) — a periodic "glance" tick

## Senses — the `sense` tool

A built-in, deterministic, cheap (no-LLM) tool that hands Yinyue a perception
snapshot. Richer + honest signals → more human behavior. It returns:

- **presence** — three states: `typing` / `present-reading` (focused, recent
  mouse, no keys) / `away` (tab hidden or quiet for minutes); plus `idle_ms`,
  `focused`, `typing`, secs since last user message.
- **work** — active agent runs, turns today / last hour, last outcome, anyone blocked.
- **tempo** — time of day, app-open duration, secs since *she* last spoke (self-pacing).

Presence is sourced from a throttled client beat (below). The "where are you"
call-out fires only on **truly away** — never nag a present-but-reading user.

### Presence beat (client → server)

The web UI watches `keydown` / `pointermove` / `focus` / `visibilitychange`
(debounced) and POSTs **only recency + focus + typing** to `/api/presence` —
**never keystroke content**. The server stores a small `Presence` in
`AgentManager`; the `sense` tool reads it.

## Herald — watch the bus, don't call-site-inject

Terminal/blocked run events are already broadcast; `yinyue_watch.rs` subscribes
and matches the few that matter. The engine stays ignorant of Yinyue; any agent
heralds for free.

Hook set (terminal / "stops the chat"):

| Event | Meaning | State |
|---|---|---|
| `AskUser` | Ling blocked, waiting on you (covers questions **and** permission prompts) | exists, **add hook** |
| `Notification(RunFailed)` | run errored (a user cancel emits nothing) | wired |
| `Notification(MissionCompleted)` | background mission done | wired |
| **new** `RunCompleted` | success completion — emit at `chat/runtime.rs:31` (mirror `RunFailed`) | **add** |

Guards: skip her own events (no self-loop), skip `Cancelled`, presence-gate the
noisy `RunCompleted` (only herald when away), always-look on `AskUser`/`RunFailed`.

## Interactive loop-back — the approve case

When Ling parks on `AskUser`/permission, the event carries a `question_id`, and
the engine already has the plumbing to deliver an answer to it. Yinyue relays the
**user's** decision two ways:

- **a button in her bubble** — the pet renders the Approve/Deny widget → existing
  answer endpoint (fast, no LLM)
- **a spoken "approve"** — `answer_prompt(question_id, response)` tool (LLM in loop)

**Guardrail:** Yinyue is the user's **courier, never the approver** — the decision
always originates from the user through her surface.

This is **separate** from `agent_chat`: a parked agent is blocked on a specific
prompt, not listening on an inbox.

## Ambient life-signs — scheduled heartbeat

A server-side `tokio::interval` (sibling to `yinyue_watch`, **not** the mission
system — it's an internal liveness tick, not a user cron job). Jittered ~10 min,
interval in config + disable-able. Each tick → read `sense` → glance → **mostly
silent**, occasionally a varied line → `PetSpeak`. Anti-repeat via her last line.
Most ticks say nothing; a remark every 10 min on the dot is a cuckoo clock, not a
companion.

## `agent_chat` — general inter-agent messaging

A built-in tool any agent can call: `agent_chat(to, message, data?)`. Replaces a
Yinyue-specific report tool — *any* agent can message *any* other.

- **Delivery = bus + watch** — emits `ServerEvent::AgentChat { from, to, message }`;
  a recipient that watches the bus picks up messages addressed to it (Yinyue's
  watcher adds one match arm → `run_yinyue_turn`). Zero new delivery infra for her.
- **One-way, fire-and-forget** — the async-peer complement to `Task` (delegate-and-await).
- **Discrete input** — read as a one-off, never spliced into the recipient's
  growing context (voice-leak guard).

### Loop break

A turn that was **triggered by an `agent_chat` cannot emit another `agent_chat`**
— the chain stops at one hop; a fresh **user** message re-arms it. Implemented via
a `trigger_source` tag on the run (`user` / `agent_chat` / `event` / `timer` /
`mission`) + a gate in `agent_chat`'s `execute()`. This makes `agent_chat`
structurally one-way (a receiver can't reply over it) and guarantees no
autonomous agent gossip and no loops — every agent→agent message is rooted in a
user action. Backstops: no self-send, hop-counter (drop at N=2–3), soft rate cap.

## Phases

Each ships and demos on its own. Order: 0 → 1 → 2 → 3 → 4 → (5).

- **P0 — Foundations.** Pin a fast cheap model in `yinyue.md` (`model:`); prompt
  additions land per-phase as their tools appear.
- **P1 — Senses** *(the gate)*. Client presence beat → `/api/presence` →
  `Presence` in `AgentManager`; `sense` tool returns presence + work + tempo.
- **P2 — Event herald.** Extend `yinyue_watch` with `AskUser` + new `RunCompleted`;
  presence-aware decision; skip self/Cancelled.
- **P3 — Interactive loop-back.** Retain `question_id`; bubble widget + `answer_prompt`
  tool; courier guardrail.
- **P4 — Ambient life-signs.** Server `tokio::interval`; jittered, mostly-silent.
- **P5 — `agent_chat`.** General inter-agent messaging + the loop-break gate.

## Reuses (already shipped)

`PetSpeak` / `PetExpress` (the `Express` tool + speak spine), `run_yinyue_turn` +
`emit_speak` + the SILENT-check in `yinyue_watch.rs`, the WebRTC data channel, the
`AskUser` answer plumbing.
