# Yinyue — the living companion

Yinyue is the user's companion — 银月, the proud fox 妖王 of 《九鼎录》 (persona: `agents/yinyue.md`) — and the face on the desktop. This spec
covers making her *proactive* and *interactive*: she reacts to what's happening,
reads whether the user is present, speaks and emotes in her own voice, and relays
between the user and the worker agents.

Ling and Yinyue are **separate, peer agents** (extensible to more). Yinyue is the
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
every turn for any session whose tool set explicitly declares `sense`, after
the `# World` block (the machine, the page in front, the apps, the phone, the
doorbell) that every session a person is in is given (`perception-spec.md` §2).
The `sense` tool still exists and returns the identical JSON, but she rarely
needs it.

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
**never keystroke content**. Skill pages beat the same way through
`shared/chat-bridge.js`, adding the `app` they show. Each beat carries a
`surface` id minted once per page load, and the engine keeps **one reading per
surface** (`engine/agent/presence.rs`): the person's state is the most present
live surface — present if ANY surface beating within 60s shows recent activity,
away only when none does. A surface silent 5 minutes is forgotten. A turn typed
anywhere (the phone included — it does not beat) stamps its own `user-turn`
surface. Beats without a `surface` (old clients, `scripts/live-check.sh`) share
one slot per app, where a blurred beat cannot clear a focused one under 10s old.
The `sense` tool and the herald watch read the aggregate.

### Heard or read — her voice only where she can't be seen

Her words are spoken aloud only when the person cannot see them; at the page
where the line shows, it is text only. One rule for everything she says —
answers, heralds, moments — decided in one place (`emit_speak` →
`speaks_aloud`, `server/api/yinyue.rs`) by the same judgement the heralds
use (`resident::triggers::user_sees`): `Sight::Screen` for "at any screen",
`Sight::Line(app)` for "at the page this line shows on". A line shows on the
page of the place it was said in — an app's page (the turn's skill), or the
main page for home and her own thread; on any other page it is folded away.
The person is at that page when one of its own surfaces reads present
(`PresenceBoard::at_page`; a typed turn names no page). A text-only line is
`voice: false` with no clauses: the stage shows the bubble and any held
gesture, never the talking loop, and fetches no audio — as when muted. When
and how often she speaks up is unchanged. The phone has no spoken voice.

### Live voice for an app's page (2.7)

An app whose page talks to a provider's live voice socket (the language
teacher's live lesson) never holds the provider's key: the engine, which holds
the keys, mints a short-lived credential for **one** session and hands the page
only that; the page then connects straight to the service. The engine names no
app; any page may ask. Providers are a declared list (`live::PROVIDERS`, one
entry each: token endpoint, model, socket address, where its key is found).
There are two services, **in this order of preference: `alibaba` (Qwen's live
audio model), then `google`**. A page takes the first one `GET /api/live` says
is ready; flipping the preference is one edit to the list. Both hand the page a
short-lived credential, never the account key.

- **`POST /api/live/token`** `{"provider": "google", "setup": {…}}` → `{provider, model,
  ws_url, token, expires}`; `ws_url` is complete — the page opens it as given.
  `setup` (optional) is the session the page wants, in the provider's own shape.
  For `alibaba` the setup is checked (an object, the size cap) and dropped: the
  token is asked with no body, lives 60 seconds, `ws_url` carries it in the
  query, the page writes its own session on the socket, and `expires` is the
  service's. For `google` the token CARRIES it: the provider uses the setup in the token and ignores what
  the socket sends afterwards, so an instruction sent only on the socket is
  dropped. The provider's entry keeps only what a page may ask (its
  instruction, the two transcripts, resumption, compression — never tools) and
  writes the model and audio output itself. One use, alive 30 minutes, to be
  used within one minute; a reconnect asks a new token with the handle in its
  setup. `expires` is the provider's when it names one, else the lifetime asked
  for. Refusals are `{"error": <code>}` with a fixed code and nothing of the
  provider's reply: `unknown_provider`, `bad_setup` (not an object, or over
  64 KiB) 400, `no_key` 409, `slow_down` 429 (five tokens a minute),
  `key_refused`, `request_refused`, `provider_down`, `unreachable`,
  `bad_answer` (each 502).
- **`GET /api/live`** → `{providers: [{id, ready, why?}]}` in the order of
  preference above, `why` one of `no_key`, `not_owner`. It calls no provider
  and starts nothing.
- **Owner only**, as hearing is: both routes are classed at chat and answer a
  caller below the owner 403 `beyond_level` (the list says `not_owner`).
- **The key and the token are never written** to a log, an event, a session row
  or an error text. Google's key is the one stored for the `gemini` provider
  (Settings → Models, found by provider and host, no model call); Qwen's is the
  one stored for a model at host `ap-southeast-1.maas.qwencloudapi.com`, under
  any provider name; scratch engines, which hold none, take
  `LINGGEN_LIVE_GOOGLE_KEY` and `LINGGEN_LIVE_ALIBABA_KEY` (`doc/cli.md`).

### Asked aloud — answered aloud (2.7, engine half built)

A person who speaks to her is not reading, so the rule above has one agreed
exception: **asked aloud, answered aloud**, also with the page in view.
Muted on this device she answers in words only — mute stays absolute — and a
typed line at the page stays text.

- **Hearing, `POST /api/stt`** (`server/api/stt.rs`, owner only, routed as
  `/api/tts` is). Body: one clip, WAV 16 kHz mono 16-bit PCM, at most 60 s —
  raw `audio/wav`, or `{"wav_b64": "…"}` as JSON (what a data-channel request
  can carry), the JSON form optionally naming `"language"`: a language code
  (`en`, `zh-Hans`) the clip is in, or `"auto"` — tell the hearing model nothing
  (an app for learners: they speak the language they learn, which neither the
  Settings language nor the last line typed says); not a code is 400
  `bad_language`, and without the field the engine decides as before
  (`language_to_hear`; `clip_language` is where the two meet). Any other media type is 415 before anything starts. Answer `{text, language, ms}`; nothing intelligible, or a clip
  that never rises above a quiet room's noise (the model is not asked: Whisper
  invents words from silence), is `{"text": ""}`; not that WAV 400; over 60 s
  413; a machine that cannot hear 503 `{error: "unavailable", reason,
  fetching, next}` with a reason of at most twenty words — said before the clip is
  looked at, so a silent clip gets it too.
- **Can she hear? `GET /api/stt`** (same door, owner only): `{"hears": true}`
  or `{"hears": false, "reason", "fetching", "next"}`, asked by the page at the press,
  before the microphone opens. It is the POST's own judgement (one function,
  `stt::hears_now`; a test holds that the two never disagree) and starts
  nothing: only a clip sent begins the next step hearing waits on, so a refused
  press that finds the model absent (or not loaded) and not on its way sends an
  empty request to start it. `next` is the engine's own word for what would help,
  so a page never reads a sentence to decide: `never` (this Mac cannot hear),
  `download` (files are missing and no fetch runs: a clip, or the empty request,
  starts a download, and `reason` names its size — the hearing model 1.6 GB, about
  4 GB with her voice), `working` (a fetch or a load runs; `fetching` is true),
  `retry` (nothing is missing: the next press only loads, or tries again). A page
  that must not download without a yes (`/shared/hold-to-talk.js` with `knock:
  false`) asks `status()`, and sends the empty request (`start()`) only from the
  person's yes.
  A fetch that failed says why and whether trying again helps (at most 20 words);
  one being retried says so (`hf_fetch::Note`; see storage-spec).
- **Can she speak? `GET /api/tts`**: `{"speaks": true, "natural": bool}`, or
  `{"speaks": false, "natural": false, "reason"}` on a machine with none of her
  voices. `natural`: the voice lane's own voice is on disk; otherwise a line is
  spoken by Kokoro (when its weights are on disk) or the system voice. It reads
  the disk and starts nothing (a test holds that its code names no synthesis, fetch
  or load). `POST /api/tts` does not fetch the model in front of a line: with Kokoro's model
  missing it speaks with the system voice and leaves the voice pack to the quiet-moment
  fetch (with the model on disk and one voice pack absent, that ~500 KB pack is still
  fetched by the line, as before). `/shared/speak-aloud.js` is the page's reader of both.
- **The engine is Whisper large-v3 turbo on MLX** (`runtime::STT_MODEL`, named
  once beside the voice model), in the voice sidecar that already speaks her
  (`mlx_sidecar.rs`, one process, one slot; `mlx_voice.py`). Same machine
  needs as her voice (Apple Silicon, 16 GB — no lane of its own). Fetched at
  first use: the first ask answers 503 "Fetching the hearing
  model", and the progress rides the `tasks` topic as her voice's does. A Mac
  that already holds the model loads it at start, after her voice is ready and
  with the pet on (`background::warm_at_start`), so the first press does not
  wait; a Mac that never fetched it does nothing at start. The
  fetch is one attempt (`runtime::Attempt`): a failure is "failed", never
  "fetching", and the next press tries again and says it is a retry. Loading
  the model is its own step with its own long wait (`op: "load"`, started when
  the fetch ends and at a press that finds it on disk but not loaded; "Getting
  ready to listen…" until then) — a hear is never the load, and waits longer
  for the sidecar's one slot than her voice does. The clip's language is told to
  the model, decided in one place (`stt::language_to_hear`): the language
  setting; under Auto, the language of the line the person last typed to her
  in her thread (`resident::language_person_typed_to_her`: Chinese or nothing;
  a spoken line is not evidence, and the first press of a day finds none).
  With neither, no language is told and the model finds it: the Mac's
  language is not evidence of what a person speaks. It is read only for a
  clip that is heard, never for a refused or silent one. Chinese carries
  a Simplified prompt (`stt_mlx::hint_of`); segments the model
  reports as no speech, low confidence or a repetition loop are not words
  (`stt_mlx::words_of`).
- **The sound is never kept or sent.** The provider is handed samples (no path,
  no URL); the sidecar gets them on stdin and gives them to the model as an
  array — no file, no log line, nothing in the session; the model is read from
  the local cache by path, never by a hub lookup. What the library writes on
  stderr during a hear is counted, never logged (`mlx_sidecar::StderrLog`;
  the words may be the person's). Locks:
  `stt::a_hearing_has_no_way_to_keep_or_send_the_sound`,
  `stt_mlx::no_sound_is_left_behind_on_disk`,
  `stt_mlx::the_sidecar_script_keeps_what_it_hears_to_itself`.
- **Her door takes `asked_aloud: true`** (`POST /api/yinyue/chat`). It is a fact
  of the turn's start: `TurnStart::Aloud` (`chat/turn_start.rs`), a person's
  words in every way a typed line is, whose line is kept as the user's row of
  her thread with `heard: true` (one field of the row, for a page that shows
  it was spoken; an app's own page that holds the microphone marks a person's
  line in its chat the same way, through `ChatRequest.heard` — `chat::helpers::heard_mark`
  decides for both, and it changes nothing else) and whose answer `speaks_aloud` (`api/yinyue.rs`, still the
  one place) says aloud unless muted. Quiet hours and the hourly limit do not
  apply to it: she was asked. A line that would be carried to an app's own chat
  (a private app named, or her stage on an app page with a chat) is not: a
  spoken one is refused at her door, 422 `{error: "not_by_voice", reason}`,
  nothing kept or run (`yinyue::door_for`, the one place that decides where a
  line goes); the page shows the reason. A typed line is carried as before,
  and follows that chat's rule. A turn replayed after a restart is not spoken.
- **A spoken answer to a waiting question** is weighed by the words as heard,
  never by what she made of them (`TurnSettings::heard`, kept only by a spoken
  start): `answer_prompt`'s `relay_selection` relays it only when those words
  are exactly one option's label — edge spaces and marks trimmed, case and
  full-/half-width folded, never a substring, never free text, never a grant
  for every folder. Anything else relays nothing; she says what she heard and
  that it must be typed or tapped. The labels are the options' own (English
  "Allow once", "Deny"), and for those two answers their Chinese is the label
  too ("允许一次", "拒绝", whatever language the page is in): one table beside
  the labels (`permission::label_in_chinese`), held to the page's string table
  by a test. No setting and no locale feeds the decision. Nothing else is
  accepted — not 好, 可以, yes, ok. A Traditional-Chinese clip ("允許一次") is
  not in the table and relays nothing. A typed answer is as it was.

## Herald — what she tells of Ling's work, decided in one place

Terminal/blocked run events are already broadcast; `server/resident/triggers.rs`
subscribes and turns the few that are news of Ling's work into one closed set,
`resident::herald::Herald`: **a reply is ready** (`Notification(RunCompleted)`),
**a run waits for the person** (`AskUser` — a question or a permission prompt),
**a run failed** (`Notification(RunFailed)`; a user cancel emits nothing).
Each is handed to one entry, `herald::tell`, which alone decides, for every
kind:

- **Whose.** A run in a session another tool started (`creator: external`,
  an `agent_run` from Claude Code or Codex) is never told — its caller has
  it — nor a session of another person's (`day_session::is_builders`). This
  is checked first, before a cooldown or a hold is spent on it. Her own turns
  are not news; her own failure rides her next woken turn (`run_fault`).
- **Where.** Only while the person is away (`user_sees(Screen)`, checked
  before her turn and again before she speaks): at a screen the page holds
  the reply, the card and the failure line. What she says goes out as her
  stage's cue alone — spoken unless muted or her line is drawn where they
  look (§ Heard or read) — and **never as a row of the thread**. Instead the
  thing it is about carries one folded mark, "Yinyue said this while you were
  away · HH:MM", her line inside: the run's last row (its reply, or its
  failure line) and the waiting question's card. The mark is set only when
  the line was heard — said aloud to a surface that holds her
  (`yinyue_line_holder`); muted, or with no such surface, it leaves none — and reaches
  the page live (`notice_told`) and on load (`told_aloud_at` /
  `told_aloud_line` on the row's meta and the listed question). The marks
  live in memory: a restart drops them, never the reply or the card.
- **The answer.** A wait's answer lives on its card. Her cue for a wait names
  the question (`question_id`), and her bubble then carries that question's
  own card — `PendingAskCard`, the one implementation of its buttons on the
  page, in a subagent's pane and in her bubble — answered through the same
  action as the page (`ui/src/lib/answerAsk.ts`: one tap at a time, a failure
  shown and the card kept) (§ Interactive loop-back). Answered anywhere, the
  card leaves her bubble. A question kept to a
  private app is told only as that the app waits, and her bubble shows no
  card for it.
- **No repeat.** One telling per waiting turn — the root run of the turn
  tree that asked (`herald/waits.rs`) — while any question of it waits: a
  second question of that turn (a subagent's prompt seconds after its
  parent's) is folded into the first telling and wakes no one; its card
  carries the same mark. Another turn's or another app's question, in the
  same session too, is its own wait, told on its own, and a mark lands only
  on the cards of the wait it was said for. A telling that came to nothing
  (silence, the answer came first, the person came back) hands the wait to
  its next question still waiting, once each; a late end of an old telling
  changes nothing newer. Once a wait is over nothing about it is said or
  written. A failure keeps its 90 s storm cooldown, and is
  told at once or not at all: held, it would ride a later turn of hers as a row.
  A fact of her own held for the person (`run_fault`) never rides a notice:
  a notice leaves no row, so the fact would be used up unrecorded.

Locks: `herald::tests::one_entry` (the notice's cue, its mark, the turn of a
wait and the gate of whose are made in `herald.rs` alone; the bus's notices
are built and handed to `tell` in `triggers.rs` only; the general doors —
`wake_to_tell`, `wake_and_speak`, `emit_speak`, her own turn — list every
caller; no glob import; every history view marks its rows), `whose` is private to
`herald.rs`, and the system tests in `tests/system/heralds.rs`.

A mission run's fault (§ below), a restart's notice (`wake_to_tell`, kept as
her row), app moments and glances are not notices of Ling's work and keep
their own paths. A phone push that carries a wait's approve/deny is not
built: the phone reads the pending list (`/api/pending-ask-user`).

**A turn the engine started for her shows nothing on any page.** Her held
runs (every origin but a person's or an asked one) are known by id as they
begin (`resident::hold_run`), and the one mapping every page reads
(`ui_events::map_server_event_to_ui_message` → `resident::shows_on_page`)
drops every event they send — status, run, token count — live; nothing of
them is a row on reload. What she says reaches the page only as her cue (and,
for a line that is not a notice, her row).

## A mission run that went wrong — a fact, not a verdict

A mission run is unattended; what went wrong in it has no one to see it. When a
run ends, the mission herald (`Notification(MissionCompleted)` →
`server/resident/mission_fact.rs`) reads the run's record in `runs.jsonl`
(`doc/mission-spec.md` § Run history) and, when no person watched it
(`attended` is not set) and something went wrong — a tool call failed, the run
failed, or it stopped at its step limit — hands her the facts:

```text
The background job "Harbor digest" ran on its own at 01:00 on 2026-11-16
(status: completed). What the engine recorded about the run:
- fetch exited with status 3; the last of its error output: "fetch: could not reach host"
- the same failure showed in each of the last 3 runs, on 3 different day(s), since 2026-11-14
```

…then the herald's usual choice: one or two sentences in her voice, or
`SILENT`. Up to three failed calls are named (each with its capped tail; a
refused call has none — its text is the engine's), the rest counted; a failed
run's error is quoted; the streak line is computed from `runs.jsonl` (the same
tool failing the same way, over runs that ran; older records without failures
end it) — the one fact a model woken for one run cannot know. A later run
that had nothing go wrong is named too. Nothing tells her it matters; nothing
is said by code.

- **Held for the person.** Missions run while nobody watches. When the person
  is at a screen (presence is not `away`) and it is not quiet hours she is
  woken at once; otherwise the fact waits (a 5 s check) until quiet hours are
  over and presence says someone is back, and only then is she woken. The wake
  is an unprompted source of its own kind (§ Guardrails): quiet hours hold it,
  the hourly ceiling neither counts it nor holds it, and nothing drops it. One
  held run per mission, the newest, its fact built from the record when she is
  woken: a mission failing every night while they are away is one wake when
  they return, its streak naming the nights. The hold is in memory — a restart
  loses it, never the record.
- A clean run, an attended run and a cancelled one hand her nothing.
- No loop, no burst: her turn is a plain agent run, not a mission run, so it
  emits no `MissionCompleted`; each finished run is handed at most once.
- Her turn runs on her own rolling thread (the kickoff is a `[HIDDEN]` row
  there). The run also leaves a `fault` line in the activity log
  (`doc/perception-spec.md` § 3), which `recent_activity` returns; her world
  block does not repeat it in this turn — the doorbell's latest line is left
  out when the newest record is a fault (`perception-spec.md` § 4). Pet off or
  not yet met: nothing is held or handed — the record, the missions page and
  the activity log still hold it.

## Interactive loop-back — the approve case

When Ling parks on `AskUser`/permission, the event carries a `question_id`, and
the engine already has the plumbing to deliver an answer to it. Yinyue relays the
**user's** decision two ways:

- **the card in her bubble** — her cue for a wait names its question, and the
  bubble (the main page's dock, the desktop pet window) draws that question's
  own card (`PendingAskCard`) → the existing answer endpoint (fast, no LLM)
- **a spoken "approve"** — `answer_prompt(question_id, response)` tool (LLM in loop)

**Guardrail:** Yinyue is the user's **courier, never the approver** — the decision
always originates from the user through her surface.

This is **separate** from `agent_chat`: a parked agent is blocked on a specific
prompt, not listening on an inbox.

## Ambient life-signs — scheduled heartbeat

A server-side timer (sibling to `yinyue_watch`, **not** the mission system —
it's an internal liveness tick, not a user cron job). A glance every **30–50
min**: 30 min plus up to 20 of jitter, fixed in `resident/ambient.rs` (start-up
adds a 4 min settle before the first wait); the Pet switch turns it off. Each
tick → read `sense` → glance → **mostly silent**, occasionally a varied line →
`PetSpeak`. Anti-repeat via her last line. Most ticks say nothing; a remark every
40 min on the dot is a cuckoo clock, not a companion.

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

Then she is woken once with everything queued (`wake_for_moment`), in her own
voice. A turn the engine started shows and keeps nothing of what she writes:
she speaks only by one `<say>…</say>` that is the last thing in her reply
(text before it is dropped; none, two, text after, an empty, broken or
over-long one says nothing), and only `resident::spoken::finalize_event`
turns that into a line the row, the live event and the voice accept.
Every kickoff the engine writes her — heralds,
ambient moments, a relayed `agent_chat`, an app moment — is a `[HIDDEN]` row
(`resident::spoken::with_contract`): kept for her model, never shown as the
user's words. Rows saved before 2026-10-14 carry no marker; the history views
know them by the spoken contract every kickoff has carried since 2026-09-08
(`is_engine_prompt`). Older rows still show. Every `PetSpeak` stamps the cooldown
(`emit_speak` → `note_spoke`), so every line she says counts, whatever
the path. The engine names no app: it carries the app's
words. Why a gate and not a wake per event: a model asked to respond always
responds, and a companion told every event talks over the game — silence is
decided here. `agents/places/desktop-pet.md` § Beside them in what they play is how she answers.
First user: Lingjing (fight outcomes, a 杀招 let go, 气血 at a quarter, too hurt
to fight).

**In the app's chat.** `session` (the app's chat session id; unknown → 400)
wakes her **at that chat's table**, a member of it (`doc/shared-session-spec.md`):
her turn runs in that session on a `[HIDDEN]` kickoff only she reads, she
reads its whole thread, and her line is a row there — `[Yinyue]` in the embed
chat, and in Ling's thread on his next turn — without starting a Ling run.
No envelope leaves nothing, and nothing runs in a chat whose skill keeps her away
(`absent_until` is read again when the wake comes). Her place there is the
skill's `place.yinyue` (else the member place), her tools what the skill names
for her (`place.yinyue.tools`). A moment with no `session` runs on her own
thread. `converse: true` (with `session`, for big moments) then gives the
session's lead **one** light turn to answer her in a line, in-world, or
`SILENT` (nothing shown, nothing kept). A light turn (`engine::light_turn`)
is one model call on the member's model and fallback chain: his soul, a short
place (the skill's description, not its SKILL.md), the chat's last few lines
and the ask — no tools, no memory, no loop; the ask is never kept, his line is
a row like any reply. It waits behind a running turn, never interrupts, never
wakes her back, and a session gets at most one exchange per **2 min**.

**Her line is all she gives.** A moment turn is *sealed*: `agent_chat` is
withheld (not offered, refused if called — `engine.withheld_tools`), so she
can't message Ling into the app's chat; `converse` is the only exchange.

**Addressed in a chat.** A message opening `@Yinyue` / `@银月` (id or a spec
`aliases:` name) goes to her, in the same session, as a **member** — seated for
good if she wasn't (`doc/shared-session-spec.md`). One thread: she reads all
of it — the user's lines, Ling's replies and his tool use (as notes, `[Ling
used Look → …]`), recall and compaction notes. The session's tool set and
`permission.json` are hers there too (a write on her turn asks the person in
that chat); a skill session's are what the skill names for her. Her reply is a
row of the session, sent to her stage (`PetSpeak`, heard only off that page —
§ Heard or read); her recall row is kept like any
member's. One agent speaks at a time (the session's turn lock); Ling reads
what she said when his turn comes. The system-prompt export takes
`agent_id=yinyue` for her seat in that session.

**Spoken to on a stage.** A stage on an app page with a chat learns the chat's
session from the page (`shared/chat-bridge.js` ⇄ the pet view, `linggen-app-chat`
messages) and sends it with `yinyue_subscribe`. While that stage holds her,
`/api/yinyue/chat` (her pet box, the desktop pet) lands in that chat as the
user's line to her, and she answers there as a member. Otherwise → her own
thread. A page frames the stage at `engineUiUrl('pet=1&stage=1')`
(`/shared/api.js`), which also resolves through linggen.dev.

**The page hears its own agent.** Another member's turn is hers alone: it never
ends the skill agent's run, question or spinner. The bridge hands `onStreamToken` /
`onStreamEnd` the skill's own agent's (the lead's) stream only
(`mount({ agentId })`); `guestStreams: true` (the page API keeps its name) adds
the other members', each call naming `info.agent`. A page's hidden
reports always go to its own agent.

## Doing what an app offers — `AppTool`

She reaches an app's action through the one `AppTool` — her own reads (`pet: true`) and what the app declares `reach: anywhere` (`skill-spec.md` § Reach), named in the prompt's index of apps. "放首轻音乐" is then two model calls: her call that emits `AppTool`, and her answer after the result, which she has — not a message to Ling, who opens the app, calls the tool and answers a third time (four calls, her never told).

What she may run is judged in one function (`skill::judge`) by the action's class and who started the turn:

- **A read** — always.
- **A change** (`tier: edit`) — only in a turn the person started by asking her (what the turn's start already says: they spoke, or said it aloud — `TurnStart::person_spoke`); in a turn she started herself (a glance, a woken note, an app moment) the answer is a reason she can word. Then, like any model's call, where the app runs — its folder, under the grants its own block declares — and under the turn's ceiling: a turn another device sent, held to a level, reaches no app (Linggen's own folder is the owner's alone). Her turns act for the owner.
- **A destructive action** — never. She cannot put a question to the person from a tool today; the reason she is given is that the person does this themselves, in the app.

## Notes — her own, in front of her next time

Without notes every "come back to this later" would need a coded condition.
With them she decides what is worth coming back to, what the note says and
when; the engine only keeps it in front of her and wakes her at the time she
set. Nothing in code decides when she writes one or whether a woken note is
worth a sentence.

- **Two tools** (`tool-spec.md`): `note_write {text, look_again?, id?}` writes
  a note, or with `id` rewrites one (text and time replaced); `note_done {id}`
  finishes one. Two, not one with an op, because their params differ (the
  app-action rule); no third, because reading needs none.
- **Kept per agent** in `~/.linggen/notes/<agent>.json` (`storage-spec.md`):
  id (`n1`, `n2`, … never reused), text (at most **200** characters), when and
  on which device it was written, an optional look-again time. At most **8**
  open notes; one more is refused with the open ones listed, so she chooses
  which to finish — nothing is dropped. The tools are listed-only: an agent has
  them when its own spec names them. Yinyue's does, and so does Ling's
  (`*` plus the two): he keeps notes too, in his own file, which ride on his
  next turn and never on hers. Only hers can carry a look-again time: on any
  other agent's note a time is refused ("Only Yinyue's notes can carry a
  time."), and no wake ever fires for another agent.
- **In front of her** at every turn of hers — her thread, a moment, a glance,
  a woken note, an app's table (whatever tools the skill names for her), a
  person's chat she is addressed in — as a per-request block beside the turn
  (`session-spec.md` § Beside the turn), never a kept row: her notes are hers,
  and a kept row is read by every member of a table. Within **2400**
  characters; nothing when she has none:

  ```
  # Your notes
  Notes you wrote for yourself; only you see them. note_done finishes one; note_write with its id rewrites it.
  - n1 · written 2h ago · look again 18:00 today: the parcel arrives tomorrow
  ```

### Look again

`look_again` takes local times: `+40m`, `+3h`, `+2d`, `HH:MM` (the next time
the clock reads it), `tomorrow HH:MM`, `YYYY-MM-DD HH:MM`. A time already
past is refused. When it comes (`server/resident/look_again.rs`, a 5 s tick):

- she gets a turn of her own on her thread, the note named as the reason — a
  **quiet turn** like a glance (`Reach::SpeakOnly`): she may speak, ask, write
  or finish notes, or end `SILENT`; the turn is offered `sense`,
  `recent_activity`, `note_write`, `note_done` and nothing else (a test lists
  them), so it never acts;
- the time is cleared and saved **before** the turn runs: it never fires
  twice; the note stays open without a time until she finishes it or sets
  another;
- times that passed while the engine was down fire once after start-up (20 s
  settle), oldest first, at least **120 s** apart;
- it waits while the user is typing (the moments gate's reading) or a turn of
  hers is running anywhere (a run of hers, or a moment's wake in flight); it
  waits, unspent, while she is off or not met.

## Guardrails — what she says unprompted

Unprompted is a glance, an app moment she was not asked for (`big` and `now`
ones included), a woken note, and a mission fault. Never held: her replies,
`asked` moments, and heralds of runs the user started (their own presence
gates stand).

| Guardrail | Setting (`[pet]`, Settings → Agents) | Default |
|---|---|---|
| A ceiling on unprompted lines in any rolling hour | `unprompted_per_hour` (0 = none) | 6 |
| Quiet hours, local time, may cross midnight | `quiet_hours` (`HH:MM-HH:MM`, empty = none) | 23:00-07:00 |

**The app in use.** Quiet hours do not hold an unasked moment from the app the
person is using: its page is the one in front (`perception::stretch::is_using`,
the reading a stretch of use is kept by) with input in the last **5 min**. Such
a moment still counts toward the hourly ceiling and can be held by it. Every
other moment, and one from the app in front after five idle minutes, waits for
the morning as before; a wake that holds moments from two apps is split, the
in-use ones going on and the rest held.

**A mission fault** (§ A mission run that went wrong) is held by quiet hours
and by nothing else: it waits through them (and for a screen) and is handed
when they end; it does not count toward `unprompted_per_hour`, the ceiling
never holds it, and it is not checked again before her voice, so nothing drops
it. A wait through quiet hours is one activity record per run (`yinyue held a
fault in Harbor digest (quiet hours)`).

Both are checked **before** her turn, so a held line is never written (at an
app's table her line would already be a row of its chat): a glance is skipped
(the machine's conditions wait, unspent, for the next one); held moments are
dropped (they would be stale by the time it passes); a woken note waits until
quiet hours end or the hour has room — one record, however many ticks. They
are checked once more before her voice, for two unprompted turns that ran at
once. The count is kept in memory: a restart starts the hour afresh.

What was held is one activity record (engine verb `hold`, by `yinyue`):
`yinyue held a look-again on n1 (quiet hours)` once per held wake, `yinyue
held app moments from dice (hourly ceiling)` at most once an hour for each
reason (moments keep coming while it holds); `detail` says the reason, the
source and until when. It rings the doorbell like any change, so it is
paced — never a record per tick or per batch.

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
