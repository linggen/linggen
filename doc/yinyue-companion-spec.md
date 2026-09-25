# Yinyue — the living companion

Yinyue is the user's spirit companion and the face on the desktop. This spec
covers making her *proactive* and *interactive*: she reacts to what's happening,
reads whether the user is present, speaks and emotes in her own voice, and relays
between the user and the worker agents.

Ling and Yinyue are **separate agents** (extensible to more). Yinyue is the
herald / front-of-house voice for the whole roster; Ling and future workers do
the technical work.

**Status:** shipped in 1.2.0. All six build phases below are live. The only
deferred item is the in-bubble Approve/Deny widget — the spoken `answer_prompt`
path covers the same case.

## The one spine

Every proactive surface funnels through one rule:

> **Yinyue observes** (an event, a scheduled glance, a relayed user input)
> **→ reads her senses** (presence + state)
> **→ decides** to speak / emote / relay / stay silent
> **→ acts** via `PetSpeak` / `Express` / `answer_prompt`.

She **always phrases her own lines** — never a hardcoded/templated string. The
trigger decides she *takes a look*; **presence governs the response**.

Inputs converge on her existing watch loop (`server/resident/`) and
`run_yinyue_turn`:

- **events** (system) — terminal/blocked run signals on the broadcast bus
- **agent_chat** (peer) — a message addressed to her from another agent
- **timer** (ambient) — a periodic "glance" tick

## Senses — the "Right now" block (and the `sense` tool)

A deterministic, no-LLM perception snapshot. Richer + honest signals → more human
behavior. It carries:

- **presence** — three states: `typing` / `present-reading` (focused, recent
  mouse, no keys) / `away` (tab hidden or quiet for minutes); plus `idle_ms`,
  `focused`, `typing`, secs since last user message.
- **work** — active agent runs, turns today / last hour, last outcome, anyone blocked.
- **tempo** — time of day, app-open duration, secs since *she* last spoke (self-pacing).

Presence is sourced from a throttled client beat (below). The "where are you"
call-out fires only on **truly away** — never nag a present-but-reading user.

### Delivery: injected, not fetched

The snapshot is rendered into the **`# Right now`** prompt block and injected on
every turn for any session whose tool set explicitly declares `sense`. The
`sense` tool still exists and returns the identical JSON, but she rarely needs it.

This is a latency decision, measured: `sense` takes no arguments and reads only
local state, so nothing the model says can change its answer — yet fetching it
cost a whole extra model round trip. A turn that called `sense` took two trips
and 5–11s, and the first trip produced no text at all; the same turn now answers
in one trip (~2–3s). Anything a tool can answer without seeing the model's output
belongs in the prompt, not behind a call.

The block is **volatile** — appended after the cached stable prefix, never inside
it, since presence changes every turn and would otherwise invalidate the prompt
cache.

### Presence beat (client → server)

The web UI watches `keydown` / `pointermove` / `focus` / `visibilitychange`
(debounced) and POSTs **only recency + focus + typing** to `/api/presence` —
**never keystroke content**. The server stores a small `Presence` in
`AgentManager`; the `sense` tool reads it.

## Herald — watch the bus, don't call-site-inject

Terminal/blocked run events are already broadcast; `server/resident/triggers.rs` subscribes
and matches the few that matter. The engine stays ignorant of Yinyue; any agent
heralds for free.

Hook set (terminal / "stops the chat"):

| Event | Meaning | State |
|---|---|---|
| `AskUser` | Ling blocked, waiting on you (covers questions **and** permission prompts) | exists, **add hook** |
| `Notification(RunFailed)` | run errored (a user cancel emits nothing) | wired |
| `Notification(MissionCompleted)` | background mission done | wired |
| **new** `RunCompleted` | success completion — emit at `chat/runtime.rs:31` (mirror `RunFailed`) | **add** |

Guards: skip her own events (no self-loop), skip `Cancelled`, presence-gate
`RunCompleted` and `AskUser` — herald only when away, checked again before she
speaks: a finished reply or a question with its buttons is already on the
screen the user is looking at (2026-09-11: she narrated every choice of a game
being played). Always-look on `RunFailed`.

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

## App moments — an app tells her, she speaks when it has gone quiet

`POST /api/yinyue/event { app, text, big?, asked?, mood?, session?, converse? }`
(`server/yinyue_moments.rs`).
An app posts what just happened as a plain fact in the user's language — a
game's loss, a hard win, a wound. **Nothing is said then.** The moments queue
(24 kept, 15 min stale) and a 5 s tick checks one gate, in code:

- the user is at the screen (a fresh, focused beat) and **not typing**;
- plain moments: idle ≥ **90 s**, no new moment for **20 s** (the app has
  settled), and she said nothing for **10 min**;
- `big` moments (a loss, an elite beaten): skip the idle wait, but the screen
  settles **6 s** and the cooldown is **3 min**.

- `asked` moments (the user asked HER — a reading, a word): woken at once,
  no quiet, no cooldown, and told silence is not an answer.

Then she is woken once with everything queued (`wake_herald`), in her own
voice, and may answer `SILENT`. Every `PetSpeak` stamps the cooldown
(`emit_speak` → `note_spoke`), so every line she says counts, whatever
the path. The engine names no app: it carries the app's
words. Why a gate and not a wake per event: a model asked to respond always
responds, and a companion told every event talks over the game — silence is
decided here. `agents/places/desktop-pet.md` § Beside them in what they play is how she answers.
First user: Lingjing (fight outcomes, a 杀招 let go, 气血 at a quarter, too hurt
to fight).

**In the app's chat.** `session` (the app's chat session id; unknown → 400)
has her spoken line also land there as a message from her — `[Yinyue]` in the
embed chat, and in Ling's context on his next turn — without starting a Ling
run. SILENT lands nothing. `converse: true` (with `session`, for big moments)
then gives the session's agent **one** hidden kickoff to answer her in a line,
in-world, or `SILENT` (nothing shown, nothing kept). It waits behind a running
turn, never interrupts, never wakes her back, and a session gets at most one
exchange per **2 min**.

**Her line is all she gives.** A moment turn and a guest turn are *sealed*:
`agent_chat` is withheld (not offered, refused if called — `engine.withheld_tools`),
so she can't message Ling into the app's chat; `converse` is the only exchange.

**Addressed in an app's chat.** A message opening `@Yinyue` / `@银月` (id or a
spec `aliases:` name) goes to her, in the same session, as a **guest**: her own
engine and tools (never the session's skill, its tools or prompt), the thread
rebuilt from the chat's visible dialogue, her reply persisted as hers and
spoken (`PetSpeak`). Her memory stays hers: her core block (who the user
is) and her recall, read by her model and never left as a row on the table;
her memory tools act for the table's session, so a skill with a
`memory-context` holds her reads and writes to that context, and her writes
are stamped with the session. Ling is not woken; what was said reaches him at the start
of his next turn (`chat/side_lines.rs`). No mention → Ling, as always.

**One conversation per app.** She reads the whole visible dialogue of an app's
chat (`chat/table.rs`): the user's lines, Ling's replies, hers, other agents' —
never a system prompt, tool call or result, `[HIDDEN]` kickoff or recall row.
Cut from the front to ~24k tokens in 32-row chunks (a stable prefix for the
cache). A guest turn's thread is it; a moment turn naming a `session` reads it
beside the kickoff for that turn only (never kept on her thread).

**Spoken to on a stage.** A stage on an app page with a chat learns the chat's
session from the page (`shared/chat-bridge.js` ⇄ the pet view, `linggen-app-chat`
messages) and sends it with `yinyue_subscribe`. While that stage holds her,
`/api/yinyue/chat` (her pet box, the desktop pet) lands in that chat as the
user's `@银月 …` line and she answers there as a guest. Otherwise → her own
thread. A page frames the stage at `engineUiUrl('pet=1&stage=1')`
(`/shared/api.js`), which also resolves through linggen.dev.

**The page hears its own agent.** A guest's turn is hers alone: it never ends
the skill agent's run, question or spinner. The bridge hands `onStreamToken` /
`onStreamEnd` the skill's own agent's stream only (`mount({ agentId })`);
`guestStreams: true` adds hers, each call naming `info.agent`. A page's hidden
reports always go to its own agent.

## `agent_chat` — general inter-agent messaging

A built-in tool any agent can call: `agent_chat(to, message)`. Replaces a
Yinyue-specific report tool — *any* agent can message *any* other.

- **Delivery is by recipient.** The tool emits `ServerEvent::AgentChat
  { from, to, message }` on the bus; `server/resident/triggers.rs` routes it:
  - **to Yinyue** → she receives it as addressed to her and *acts on it* —
    speaks (`PetSpeak`), moves (`Express`), or stays silent.
  - **to a chat agent** (Ling, …) → it lands in that agent's chat as a
    `[Sender]: …` message and runs the agent's turn, so it responds there. The
    target session is the one the user is viewing (`set_view_context`), else the
    agent's latest top-level session.
- **One-way, fire-and-forget** — the async-peer complement to `Task` (delegate-and-await).
- **Discrete input** — read as a one-off, never spliced into a recipient's
  growing context (voice-leak guard).

### Loop break

A turn that was **triggered by an `agent_chat` cannot emit another `agent_chat`**
— the chain stops at one hop; a fresh **user** message re-arms it. Implemented via
a `trigger_source` tag on the run (`user` / `agent_chat` / `event` / `timer` /
`mission`) + a gate in `agent_chat`'s `execute()`. This makes `agent_chat`
structurally one-way (a receiver can't reply over it) and guarantees no
autonomous agent gossip and no loops — every agent→agent message is rooted in a
user action. Backstops: no self-send, hop-counter (drop at N=2–3), soft rate cap.

## Build phases (all shipped, 1.2.0)

Built and verified in order 0 → 5:

- **P0 — Foundations.** Pin a fast cheap model in `yinyue.md` (`model:`); prompt
  additions land per-phase as their tools appear.
- **P1 — Senses.** Client presence beat → `/api/presence` → `Presence` in
  `AgentManager`; `sense` tool returns presence + work + tempo.
- **P2 — Event herald.** `yinyue_watch` matches `AskUser` + new `RunCompleted`;
  presence-aware decision; skip self/Cancelled.
- **P3 — Interactive loop-back.** Retain `question_id`; `answer_prompt` tool;
  courier guardrail. (Bubble Approve/Deny widget deferred.)
- **P4 — Ambient life-signs.** Server `tokio::interval`; jittered, mostly-silent.
- **P5 — `agent_chat`.** General inter-agent messaging + the loop-break gate.

Refinements shipped on top of the plan:

- **`Express` is pet-scoped** — excluded from the `*` wildcard, granted only to
  an agent that lists it. Workers ask Yinyue via `agent_chat` to drive the avatar.
- **`agent_chat` is bidirectional** — a message to a chat agent renders in its
  chat (`[Sender]: …`) and runs its turn; routed to the user's focused session.
- **A message to Yinyue is hers to act on** — "Dance!" → `Express(dance)`, not a
  relay-to-user.

## Reuses (already shipped)

`PetSpeak` / `PetExpress` (the `Express` tool + speak spine), `run_yinyue_turn` +
`emit_speak` + the SILENT-check in `server/resident/spoken.rs`, the WebRTC data channel, the
`AskUser` answer plumbing.
