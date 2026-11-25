---
type: spec
reader: Coding agent and users
guide: |
  Product specification — describe what the system should do and why.
  Keep it brief. Aim to guide design and implementation, not document code.
  Avoid implementation details like function signatures, variable types, or code snippets.
---

# Yinyue — who she is, where she appears, what she says and does

> **How to keep this spec**
> - It records WHAT Yinyue is, where she shows up and the rules each surface obeys — no history, no code, no function signatures.
> - Detail lives where it executes; point there (last section), do not copy it.
> - A change in behaviour updates the matching line here in the same commit.
> - A claim you cannot find in the code does not belong here. Stay under ~560 lines.

Two docs: **this one** — who she is, where she appears, what she says and
does, and when she speaks unprompted; [perception-spec.md](perception-spec.md) —
what every agent (Ling and Yinyue) knows without being told (presence, the
world block, the activity log, conditions).

Yinyue is Linggen's companion: 银月, the proud fox 妖王 of Qingqiu in Linggen's
own story 《九鼎录》 (Lingjing), one tail left, power and memory mostly gone.
The same character on every surface — Mac, phone, game. She lives with the
user, remembers them, and leaves the work to Ling.

## Two agents

Peer agents (`agent-spec.md`), one of each on every device (`one-ling-spec.md`):

- **`ling`** — the worker. Answers every message and does every task, adapting
  to any context through skills.
- **`yinyue`** — the companion. Speaks on her own moments or when addressed
  `@Yinyue`; runs no tasks and passes work to Ling. She is not Ling's master.

Definition: `agents/yinyue.md` (shipped; her persona, tool list and the
`met_when` condition that says she has been met).

## Persona

Proud, haughty, dry, teasing — never servile, never sweet, never a
cheerleader. 本王 in Chinese; "this queen" now and then in English. Care shows
as orders and deeds; one softer word, then she covers it. Haughty, never rude:
money, health and real loss get a straight line. Pride never blocks help.
Short, spoken prose — her words are often read aloud. When all is well she
need not speak. Her backstory goes no further than the place carries it
(Lingjing holds the specifics).

- She addresses the person by the name in core memory. With none yet, she uses
  a stand-in and says so lightly; the moment a name is given she writes it to
  core memory and drops the stand-in. Another form ("Master") only if asked.
- She reads locale and hour from the Environment block every agent's prompt
  carries (timezone from `/etc/localtime`, locale from `LANG`).

## Onboarding

She has to be *met* before she appears. The `met_when` in `yinyue.md` names a
skill file and path (today Lingjing's `companion.joined`); until the engine
reads it true, the desktop shell opens no pet window and the tray stays plain.

## Surfaces

One identity, one renderer, one event vocabulary; the host decides how she is
presented.

- **Desktop pet** — a transparent, borderless, always-on-top window
  (240×420, bottom-right) in the shell (`linggen-app/shell/src/pet.rs`), opened
  for every branded app with `[features] pet`. It loads the **same** page the
  browser serves (`?pet=1`, `PetStage`) over HTTP — no bundled copy. Visible
  while any of its app's windows has focus; "Always on top" (tray menu) keeps
  it up; Settings can disable her. Drag by body; a click opens her talk input.
- **In-page overlay** — the same renderer inside the core web UI when no shell
  is present (plain browser, remote web), confined to the page. Inside the
  shell (`inShell()`, `ui/src/lib/appHost.ts`) it never shows: never both.
- **Menubar** — see below. App-only.
- **Chat** — her thread and any session she is seated in (below).
- **Voice** — see below.

Both overlays are click-through except on her body. Ambient animation (blink,
breathe, idle) is client-side — never an engine event, never a model call.

### One event spine

Every surface renders one engine vocabulary: speech and expression cues
(`pet_speak`, `pet_express`) over the WebRTC control channel. The cue is small;
audio is pulled from `/api/tts` clause by clause.

### One device, one Yinyue

A device shows exactly one Yinyue body, and that one carries her voice; every
other place she appears on that device is silent and shows her words as text.
Per device, not per person: she has a voice on each device.

Every surface that can render her — web tab, pet window, a stage inside an app
page — subscribes (`yinyue_subscribe {stage}`); the engine keeps an ordered
registry and tells each peer whether it holds her (`yinyue_present`); only the
holder gets `pet_speak` / `pet_express`. The holder is the first surface with a
**stage** (one that stands her in a place, e.g. a game scene at
`/?pet=1&stage=1`), else the first to arrive; release or disconnect promotes
the next. Two cases the engine cannot see are the surface's own duty: a phone
with the Linggen app and a browser tab keep voice in the app; at a game table
other players' Yinyue are silent figures. (`ServerState::yinyue_*`;
`useYinyuePresenter`.)

An app page can say "no body here" — the app-mode message
`{type: 'linggen:companion', companion: 'hidden'}` (`skill-spec.md` § App-mode).
While that page is the desktop app's tab in front, the app's window holds her
as a bodyless stage so the corner draws nothing; `shown`, a tab change,
settings over it or the page going away hands her back (`AppTabs.tsx`).

### Mute

A person mutes her on the device they type on. Muted, she still writes every
line — bubble, chat — and no audio is made or played, until turned back on.

- **Mac:** `[pet] muted` in config. Setting it sends `pet_voice {muted}` to
  every surface so whichever is playing stops. `pet_speak` carries `voice`;
  muted, or at the page the line shows on (§ Heard or read), the
  holder shows the bubble and fetches no `/api/tts`.
  - `/mute` and `/unmute` are caught before any model in `dispatch_turn` (every
    Mac chat, including a Ling chat relayed from the phone) and in
    `/api/yinyue/chat`; the chat gets a system line.
  - Plain words ("be quiet", "静音") go to the `Voice {muted}` tool, which every
    agent has, including a skill's restricted tool set.
  - Settings → General → Pet → Mute Voice sets the same flag.
- **Phone:** its own switch (`yinyue.muted`); `/mute` and `/unmute` are caught
  in the chat screen and never sent. The phone has no spoken voice yet. Plain
  words typed in the phone's Ling tab reach the Mac and mute the Mac.

### Menubar

A macOS tray item (`linggen-app/shell/src/menubar.rs`, `[features] menubar`)
is her persistence anchor: present with no app window open, and where she is
summoned from. Before she is met only the background launcher has a plain
tray (app icon; Open / Settings / Quit); once met, the same tray takes her face
and menu (Show / Hide Yinyue, Always on top, Settings…, Quit).

The face is a small 2D sprite pre-rendered from the same VRM and cycled by a
timer — no webview needed. Today it is local only: **rest** (blinks every ~4 s)
and **sleep** (after 3 min unfocused; wakes on app focus or a tray click).
Click summons or dismisses her. No model call ever animates it.

### Voice

Spoken line, in order of preference — the first whose files are on disk, with
`[uses] speak` naming a preferred one:

0. **Linggen Cloud's Serena** (qwen3-tts-flash through linggen.dev), first for a
   subscriber and the owner alone; the local voices stay the free, offline choice
   and take over when it is refused or unreachable (`tts::Source::Cloud`).
1. **Qwen3-TTS** on the GPU (MLX, in the shared voice sidecar), speaker from
   `[pet] voice`. "Natural" is exactly this voice being on disk.
2. **Kokoro-82M** (candle, `any-tts`; the `kokoro` build feature, default on).
   Its voice id carries the language: Han characters pick Mandarin, else English.
3. **macOS `say`**, the system voice.

`GET /api/tts` says whether she can speak and whether naturally, starting
nothing; `POST /api/tts {text, voice?}` returns a WAV and fetches no model in
front of a line. Hearing (STT): on a Mac, Apple's on-device speech recognition;
elsewhere Whisper large-v3 turbo in the same sidecar (§ Heard or read).

A line is spoken in short clauses (`api/tts_text.rs`): the cue carries them,
the surface fetches each and plays the first while the next is made. The bubble
shows the original text; only the spoken copy drops quotes and a leading
「……」 and turns 「——」/「……」 into pauses. Whether a line is spoken at all is
§ Heard or read.

## Session and memory

Her daily thread is an ordinary Linggen session: same engine, persistence,
recall and compaction as Ling's, through the one turn core `run_session_turn`
(`chat/handler.rs`). What differs: her entry triggers (events, avatar tap), her
output sink (spoken, not a chat panel), and her tool set — the narrow list in
`yinyue.md`. Elsewhere she is a member at another's table
(`shared-session-spec.md`), reading that session's thread and using its tools.

- **Which session.** With the daily session on (default), her thread is the
  person's day session, where she is seated with Ling (`day_session.rs`,
  `one-ling-spec.md`). With it off, rolling ids `sess-yinyue-YYYY-MM-DD`, with
  a `-N` segment when the day's context passes 0.7 of the soft token limit
  (`resident/session_roll.rs`). A fresh thread gets a one-line `Previously:`
  seed from the prior one; memory carries the rest.
- **Persistence.** `~/.linggen/sessions/<id>/messages.jsonl` + meta; a
  restart resumes the same thread.
- **Memory.** An owner session sharing Ling's memory (rules:
  [memory-spec.md](memory-spec.md)). Core is always loaded; per-turn capture
  writes episodic. Her session stands at `~/.linggen`, no project, so her
  recall sees rows about the person.
- **Snappy turns.** Lean recall (`[pet] recall_count` 1, `recall_min_score`
  0.8), a live prompt capped at the last 10 messages, and a prompt that says
  the relevant memory is already injected.
- **Model.** `[pet] model`: `auto` = Linggen Cloud for signed-in users, else
  the engine default; or a pinned id.

# What she says and does

Ling and Yinyue are peer agents; Yinyue is the herald and front-of-house voice,
Ling does the work.

## The one spine

> **Something happens** (a run event, a scheduled glance, an app moment, a
> message from a peer) **→ she reads what she knows → decides** to speak, emote,
> relay or stay silent **→ acts** (`pet_speak`, `Express`, `answer_prompt`).

She **always words her own lines** — never a templated string. The trigger
decides she looks; the rules in perception-spec § 5 decide whether she may.
Inputs converge on the watch loop (`src/server/resident/`): **events** on the
broadcast bus, **`agent_chat`** from a peer, a **timer** glance, **app moments**.

Her turns run as plain agent runs on her thread, not missions. A turn the
engine started for her shows nothing on any page except her cue (and, for a
line that is not a notice, her row): her held runs are known by id as they
begin (`resident::hold_run`) and `resident::shows_on_page` drops every event
they send. Every kickoff the engine writes her is a `[HIDDEN]` row
(`spoken::with_contract`): kept for her model, never shown as the user's words.
She speaks from such a turn only by one `<say>…</say>` that is the last thing in
her reply, or `SILENT`; anything else says nothing (`spoken::finalize_event`).

## Heard or read — her voice only where she can't be seen

Her words are spoken aloud only when the person cannot see them; at the page
where the line shows, it is text only. One rule for everything she says,
decided in one place (`emit_speak` → `speaks_aloud`, `api/yinyue.rs`) by the
judgement the heralds use (`resident::triggers::user_sees`): `Sight::Screen`
("at any screen") or `Sight::Line(app)` ("at the page this line shows on" — an
app's page for the turn's skill, else the main page; elsewhere it is folded
away). The person is at a page when one of its surfaces reads present
(`PresenceBoard::at_page`; a typed turn names no page). A text-only line is
`voice: false` with no clauses: bubble and any held gesture, no talking loop,
no audio fetched — as when muted. The phone has no spoken voice.

**Asked aloud → answered aloud.** The agreed exception: a person who speaks to
her is not reading, so she answers aloud even with the page in view. Muted, she
answers in words only — mute stays absolute — and a typed line at the page stays
text.

- **Hearing, `POST /api/stt`** (owner only): one clip, WAV 16 kHz mono 16-bit,
  ≤ 60 s — raw `audio/wav` or JSON `{"wav_b64", "language"?}` (`language`: a code
  like `en` / `zh-Hans`, or `"auto"`). Answer `{text, language, ms}`; nothing
  intelligible, or a clip never above a quiet room's noise, is `{"text": ""}`.
  Errors: 400 not that WAV / `bad_language`, 413 over 60 s, 415 other media
  type, 503 `{error:"unavailable", reason, fetching, next}` (reason ≤ 20 words).
- **`GET /api/stt`** → `{"hears": true}` or `{"hears": false, reason, fetching,
  next}`, asked at the press before the mic opens; starts nothing (one judgement,
  `stt::hears_now`, shared with the POST). `next` is the engine's word: `never`,
  `download` (a clip or an empty request starts it; the model is 1.6 GB, about
  4 GB with her voice), `downloading` (a Mac: a language's model is being put
  on it), `working`, `retry`. A page that must not download without
  a yes (`shared/hold-to-talk.js`, `knock: false`) sends the empty request only
  from the person's yes.
- **Engine on a Mac:** Apple's own on-device speech recognition, no choice
  (`runtime::hearing_by_apple` is the one answer; Settings shows one fact line).
  `ling-hear` (Swift, `apple_speech/`, built by `build.rs`, carried inside
  `ling`) runs as its own process per clip — samples on stdin, one JSON line
  out — so a failure ends one hearing, never the engine. On the device only,
  never Apple's servers. A language this Mac lacks answers 503 `downloading`
  and that press starts the install (macOS 26) or names where to add it
  (older). A clip that names no language is heard in the first ready Mac
  language, else English. A Whisper copy already downloaded is left alone.
- **Engine elsewhere:** Whisper large-v3 turbo on MLX (`runtime::STT_MODEL`) in the voice
  sidecar that speaks her (`mlx_sidecar.rs`, one process, one slot); same machine
  needs as her voice (Apple Silicon, 16 GB). Fetched at first use; a Mac that
  holds it loads it at start. Language told to the model is decided in one place
  (`stt::language_to_hear`): the language setting; under Auto the language of the
  line the person last typed to her; else none. Chinese carries a Simplified
  prompt; no-speech, low-confidence and repetition-loop segments are not words.
- **The sound is never kept or sent**: handed over as samples on stdin, no file,
  no log line, nothing in the session; stderr from a hear is counted, never
  logged. Locks: `stt::a_hearing_has_no_way_to_keep_or_send_the_sound`,
  `stt_mlx::no_sound_is_left_behind_on_disk`.
- **`GET /api/tts`** → `{"speaks", "natural"}` or `{"speaks": false, reason}`
  (which voice: yinyue-spec § Voice); reads the disk, starts nothing.
- **Her door takes `asked_aloud: true`** (`POST /api/yinyue/chat`): the turn
  starts as `TurnStart::Aloud` — a person's words like a typed line, kept as the
  user's row with `heard: true`, answered aloud unless muted. Quiet hours and the
  hourly ceiling do not apply: she was asked. A line that would be carried to an
  app's own chat is refused if spoken, 422 `not_by_voice` (`yinyue::door_for`
  decides where a line goes). A turn replayed after a restart is not spoken.
- **A spoken answer to a waiting question** is weighed by the words as heard
  (`TurnSettings::heard`): `answer_prompt` relays it only when those words are
  exactly one option's label (edges trimmed, case and width folded, never a
  substring, never free text); Chinese for "Allow once" / "Deny" ("允许一次",
  "拒绝") is in one table (`permission::label_in_chinese`). Anything else relays
  nothing; she says what she heard and that it must be typed or tapped.

## Live voice for an app's page

An app whose page talks to a provider's live voice socket (the language
teacher's lesson) never holds the key: the engine mints a short-lived credential
for **one** session and hands the page only that. The engine names no app.
Providers are a declared list (`live::PROVIDERS`), in preference **`alibaba`**
(Qwen live audio), then **`google`**; a page takes the first `GET /api/live` says
is ready.

- **`POST /api/live/token {provider, setup?}`** → `{provider, model, ws_url,
  token, expires}`; `ws_url` is complete. For `alibaba` the setup is checked
  (object, size cap) and dropped; the token lives 60 s and the page writes its
  own session on the socket. For `google` the token carries the setup (only what
  a page may ask — instruction, transcripts, resumption, compression, never
  tools); one use, 30 min, used within a minute. Refusals are `{"error": code}`
  with no provider text: `unknown_provider`, `bad_setup` (not an object or
  > 64 KiB) 400, `no_key` 409, `slow_down` 429 (five tokens a minute),
  `key_refused`, `request_refused`, `provider_down`, `unreachable`, `bad_answer`
  (502). **Who pays** is decided in one function (`live::find_source`): the
  person's own key first, else — a subscriber, `alibaba` only — Linggen Cloud:
  linggen.dev mints the 60 s key per lesson and the page gets the same answer
  plus `max_minutes` (lesson length cap, kept by the page) and `lessons_left`
  (this month; absent for the developer account). Cloud adds refusals
  `not_subscribed` 402, `lessons_used` 429, `daily_cap` 429.
- **`GET /api/live`** → `{providers: [{id, ready, via?, why?}]}`; `via` =
  `own_key` or `linggen_cloud` when ready, `why` = `no_key` or `not_owner`. A
  subscriber with no own key reads ready `via: linggen_cloud`. Calls no provider
  (may read the account's entitlement, 2 s at most).
- **Owner only**: below the owner, 403 `beyond_level`.
- **Key and token are never written** to a log, event, session row or error.
  Google's key is the one stored for the `gemini` provider; Qwen's the one for a
  model at host `ap-southeast-1.maas.qwencloudapi.com`; scratch engines take
  `LINGGEN_LIVE_GOOGLE_KEY` / `LINGGEN_LIVE_ALIBABA_KEY` (`doc/cli.md`).

## Herald — what she tells of Ling's work, decided in one place

`resident/triggers.rs` turns the few bus events that are news of Ling's work into
one closed set, `resident::herald::Herald`: **a reply is ready**
(`Notification(RunCompleted)`), **a run waits for the person** (`AskUser` — a
question or permission prompt), **a run failed** (`Notification(RunFailed)`; a
user cancel emits nothing). Each goes to one entry, `herald::tell`, which alone
decides, for every kind:

- **Whose.** A run in a session another tool started (`creator: external`, an
  `agent_run` from Claude Code or Codex) is never told, nor another person's
  session (`day_session::is_builders`); checked first. Her own turns are not news.
- **Where.** Only while the person is away (`user_sees(Screen)`, checked before
  her turn and again before she speaks). What she says goes out as her stage's
  cue alone — spoken unless muted or drawn where they look — and **never as a row
  of the thread**. The thing it is about carries one folded mark, "Yinyue said this
  while you were away · HH:MM": the run's last row and the waiting question's card.
  Set only when the line was heard aloud by a surface that holds her; it reaches
  the page live (`notice_told`) and on load (`told_aloud_at` / `told_aloud_line`).
  Marks live in memory: a restart drops them, never the reply or card.
- **The answer.** A wait's answer lives on its card. Her cue names the question
  (`question_id`) and her bubble carries that question's own card
  (`PendingAskCard`, the one implementation of its buttons, answered through
  `ui/src/lib/answerAsk.ts`); answered anywhere, the card leaves her bubble. A
  question kept to a private app is told only as "the app waits", no card.
- **No repeat.** One telling per waiting turn (the root run of the turn tree,
  `herald/waits.rs`); a second question of that turn is folded into the first.
  Another turn's or app's question is its own wait. A telling that came to nothing
  hands the wait to its next waiting question, once each. Once a wait is over
  nothing about it is said. A failure keeps a 90 s storm cooldown and is told at
  once or not at all.
- **Her own failure** is never a wake (her model may be what failed, and waking her
  would loop): it is held as a fact and rides her next unprompted turn on her own
  thread (`resident::run_fault`); while her model is down, nothing is said. A
  failure of another agent wakes her at once, as facts she words — never a
  sentence of the engine's.

Locks: `herald::tests::one_entry` (the notice's cue, mark, wait turn and the
gate of whose are made in `herald.rs` alone; the bus's notices are built and
handed to `tell` in `triggers.rs` only), `whose` private to `herald.rs`, system
tests in `tests/system/heralds.rs`. A mission fault, a restart's notice
(`wake_to_tell`, kept as her row), app moments and glances are not notices of
Ling's work and keep their own paths. A phone push carrying a wait's approve/deny
is not built; the phone reads `/api/pending-ask-user`.

## A mission run that went wrong — a fact, not a verdict

A mission run is unattended; what went wrong has no one to see it. On
`Notification(MissionCompleted)`, `resident/mission_fact.rs` reads the run's
record in `runs.jsonl` (`mission-spec.md` § Run history) and, when no person
watched it and something went wrong — a tool call failed, the run failed, or it
hit its step limit — hands her the facts:

```text
The background job "Harbor digest" ran on its own at 01:00 on 2026-11-16
(status: completed). What the engine recorded about the run:
- fetch exited with status 3; the last of its error output: "fetch: could not reach host"
- the same failure showed in each of the last 3 runs, on 3 different day(s), since 2026-11-14
```

She answers in one or two sentences in her voice, or `SILENT`. Up to three failed
calls are named (each with its capped tail; a refused call has none), the rest
counted; a failed run's error is quoted; the streak line is computed from
`runs.jsonl` — the one fact a model woken for one run cannot know. Nothing tells
her it matters; nothing is said by code.

- **Held for the person.** With the person at a screen (presence not `away`) and
  not in quiet hours she is woken at once; otherwise the fact waits (5 s check)
  until quiet hours are over and someone is back. One held run per mission, the
  newest, its fact built when she is woken. The hold is in memory.
- A clean run, an attended run and a cancelled one hand her nothing.
- No loop: her turn is not a mission run, so it emits no `MissionCompleted`; each
  finished run is handed at most once.
- Her turn runs on her own thread. The run also leaves a `fault` activity record;
  the doorbell's `latest` line is left out for that turn (perception-spec § 4).
  Pet off or not yet met: nothing is held or handed.

Guardrails for held faults: perception-spec § 5.

## Interactive loop-back — the approve case

When Ling parks on `AskUser` or a permission prompt, the event carries a
`question_id`. Yinyue relays the **user's** decision two ways: **the card in her
bubble** (fast, no model; the existing answer endpoint) or **a spoken "approve"**
through the `answer_prompt(question_id, response)` tool. She is the user's
**courier, never the approver** — the decision always originates from the user
through her surface. This is separate from `agent_chat`: a parked agent is
blocked on a specific prompt, not listening on an inbox.

## Ambient glance

A server timer (`resident/ambient.rs`, not the mission system): a glance every
30 min plus up to 20 min of jitter (4 min settle at start-up); the Pet switch
turns it off. Each tick she reads the room and is **mostly silent**, occasionally
a varied line; anti-repeat via her last line. Unprompted rules: perception-spec § 5.
A quiet glance (`Reach::SpeakOnly`) speaks and offers; it never acts.

## App moments — an app tells her, she speaks when it has gone quiet

`POST /api/yinyue/event { app, text, big?, asked?, mood?, session?, converse? }`
(`src/server/yinyue_moments.rs`). An app posts what just happened as a plain fact
in the user's language (a game's loss, a hard win, a wound). **Nothing is said
then.** Moments queue (24 kept, 15 min stale) and a 5 s tick checks one gate:

- the person is at the screen (fresh, focused beat) and **not typing**;
- plain moments: idle ≥ **90 s**, no new moment for **20 s**, and she has said
  nothing for **10 min**;
- `big` moments: no idle wait, the screen settles **6 s**, cooldown **3 min**;
- `asked` moments (the user asked HER — a reading, a word): woken at once, no
  quiet, no cooldown, and told silence is not an answer.

She is then woken once with everything queued (`wake_for_moment`), in her own
voice, under the `<say>` contract above. Every line she says stamps the cooldown
(`emit_speak` → `note_spoke`). The engine names no app; a gate and not a wake per
event because a model asked to respond always responds, and a companion told
every event talks over the game. How she answers: `agents/places/desktop-pet.md`.

**In the app's chat.** `session` (the app's chat session id; unknown → 400) wakes
her at that chat's table as a member (`shared-session-spec.md`): her turn runs in
that session on a `[HIDDEN]` kickoff, reads its whole thread, and her line is a
row there (`[Yinyue]` in the embed chat, and in Ling's thread on his next turn)
without starting a Ling run. Nothing runs in a chat whose skill keeps her away
(`absent_until`). Her place is the skill's `place.yinyue`, her tools what it names
(`place.yinyue.tools`). A moment with no `session` runs on her own thread.
`converse: true` (with `session`, for big moments) gives the session's lead **one**
light turn (`engine::light_turn`: one model call on the member's model, his soul,
a short place, the chat's last few lines — no tools, no memory, no loop) to answer
her in a line, in-world, or `SILENT`; at most one exchange per **2 min**, never
interrupting a running turn, never waking her back.

**Her line is all she gives.** A moment turn is sealed: `agent_chat` is withheld
(`engine.withheld_tools`), so she cannot message Ling into the app's chat.

**Addressed in a chat.** A message opening `@Yinyue` / `@银月` (id or `aliases:`)
goes to her in the same session as a **member**, seated for good. One thread: she
reads all of it — the user's lines, Ling's replies and his tool use (as notes),
recall and compaction notes. The session's tool set and `permission.json` are hers
there too (a write on her turn asks the person in that chat); in a skill session
the tools are what the skill names. Her reply is a row of the session, sent to her
stage (`pet_speak`, heard only off that page); one agent speaks at a time (the
turn lock).

**Spoken to on a stage.** A stage on an app page with a chat learns the chat's
session from the page (`shared/chat-bridge.js` ⇄ the pet view) and sends it with
`yinyue_subscribe`. While that stage holds her, `/api/yinyue/chat` lands in that
chat as the user's line and she answers there as a member; otherwise on her own
thread. A page frames the stage at `engineUiUrl('pet=1&stage=1')`
(`shared/api.js`).

**The page hears its own agent.** Another member's turn never ends the skill
agent's run, question or spinner: the bridge hands `onStreamToken` / `onStreamEnd`
the lead's stream only; `guestStreams: true` adds the others', each naming
`info.agent`. A page's hidden reports go to its own agent.

## Doing what an app offers — `AppTool`

She reaches an app's action through the one `AppTool` — her own reads
(`pet: true`) and what the app declares `reach: anywhere` (`skill-spec.md`
§ Reach), named in the prompt's index of apps. "放首轻音乐" is two model calls: her
`AppTool`, and her answer after the result. What she may run is judged in one
function (`skill::judge`) by the action's class and who started the turn:

- **A read** — always.
- **A change** (`tier: edit`) — only in a turn the person started by asking her
  (`TurnStart::person_spoke`); in a turn she started herself the answer is a reason
  she can word. Then, like any call, under where the app runs (its folder and
  grants) and the turn's ceiling.
- **A destructive action** — never; the person does that themselves in the app.

## Notes — her own, in front of her next time

With notes she decides what is worth coming back to, what it says and when; the
engine only keeps it in front of her and wakes her at the time she set.

- **Two tools** (`tool-spec.md`): `note_write {text, look_again?, id?}` writes a
  note, or with `id` rewrites one; `note_done {id}` finishes one. Listed-only: an
  agent has them when its spec names them. Yinyue's does, and Ling's — his notes
  ride on his next turn, never hers; only hers can carry a look-again time (on
  another agent's note a time is refused, and no wake fires for them).
- **Kept per agent** in `~/.linggen/notes/<agent>.json` (`storage-spec.md`): id
  (`n1`, `n2`, … never reused), text (≤ **200** chars), when and on which device
  written, optional look-again time. At most **8** open notes; one more is refused
  with the open ones listed — nothing is dropped.
- **In front of her** at every turn of hers (her thread, a moment, a glance, a
  woken note, an app's table, a chat she is addressed in) as a per-request block
  beside the turn (`session-spec.md` § Beside the turn), never a kept row, within
  **2400** chars, nothing when she has none:

  ```
  # Your notes
  Notes you wrote for yourself; only you see them. note_done finishes one; note_write with its id rewrites it.
  - n1 · written 2h ago · look again 18:00 today: the parcel arrives tomorrow
  ```

**Look again.** `look_again` takes local times: `+40m`, `+3h`, `+2d`, `HH:MM`,
`tomorrow HH:MM`, `YYYY-MM-DD HH:MM`; a past time is refused. When it comes
(`resident/look_again.rs`, 5 s tick) she gets a turn of her own on her thread, the
note named as the reason — a quiet turn offered `sense`, `recent_activity`,
`note_write`, `note_done` and nothing else. The time is cleared and saved
**before** the turn runs (never fires twice). Times that passed while the engine
was down fire once after a 20 s settle, oldest first, ≥ **120 s** apart. It waits
while the user is typing or a turn of hers is running, and (unspent) while she is
off or not met.

## `agent_chat` — inter-agent messaging

A built-in tool: `agent_chat(to, message)`; any agent may message any other.
Delivery is by recipient (`ServerEvent::AgentChat`, routed in `triggers.rs`):

- **to Yinyue** → she receives it as addressed to her and acts: speaks, moves
  (`Express`), or stays silent. "Dance!" is `Express(dance)`, not a relay.
- **to a chat agent** (Ling, …) → it lands in that agent's chat as `[Sender]: …`
  and runs its turn; the target is the session the user is viewing, else the
  agent's latest top-level session.
- **One-way, fire-and-forget** — the async complement to `Task`. A discrete input,
  never spliced into the recipient's growing context.
- **Loop break:** a turn triggered by an `agent_chat` cannot send another — the
  chain stops at one hop; a fresh user message re-arms it (a trigger-source gate
  in the tool). No autonomous agent gossip; every agent→agent message is rooted
  in a user action.

`Express` is pet-scoped: excluded from the `*` wildcard, granted only to an agent
that lists it; workers ask Yinyue via `agent_chat` to drive the avatar.

## Speaking unprompted

What she perceives ([perception-spec.md](perception-spec.md)) lets her start a sentence; it does not make every observation
worth one. Silence is the default.

**What "unprompted" is.** A glance (the ambient tick), an app moment she was not
asked for (`big` and `now` included), a woken note, and a mission fault. Never
held by the rules below: her replies, `asked` moments, and notices of runs the
person started (their own presence gates stand).

### The ceiling and quiet hours

| Guardrail | Setting (`[pet]`, Settings → Agents) | Default |
|---|---|---|
| Most unprompted lines in any rolling hour | `unprompted_per_hour` (0 = none) | 6 |
| Quiet hours, local, may cross midnight | `quiet_hours` (`HH:MM-HH:MM`, empty = none) | 23:00-07:00 |

- Both are checked **before** her turn, so a held line is never written (at an
  app's table it would already be a row of its chat), and once more before her
  voice, for two unprompted turns that ran at once. A glance is skipped (the
  machine's conditions wait, unspent); held moments are dropped (stale by then);
  a woken note waits until quiet hours end or the hour has room. The count is in
  memory: a restart starts the hour afresh.
- **The app in use.** Quiet hours do not hold an unasked moment from the app the
  person is using — its page in front with input in the last 5 min
  (`perception::stretch::is_using`). Such a moment still counts toward, and can
  be held by, the hourly ceiling. Other moments wait for the morning; a wake
  holding moments from two apps is split.
- **A mission fault** is held by quiet hours and a screen only: it does not count
  toward the ceiling, the ceiling never holds it, and nothing drops it.
- **Held is recorded**: one activity record (verb `hold`, by `yinyue`), once per
  held wake, or at most once an hour per reason for moments; `detail` gives the
  reason, source and until when. It rings the doorbell, so it is paced.


Conditions she may raise are noticed by perception ([perception-spec.md § 5](perception-spec.md)); config `[pet]`, gates in `src/server/resident/`.

## Open / next

- **Menubar daemon tier** — `talk` and mood faces (joy / worried / alert) fed
  by the voicing webview relaying a cue to the shell over Tauri IPC. No code.
- **Singleton roaming pet** — one pet owned by the daemon (a lease), a
  full-screen click-through overlay, walk/climb clips, climbing Linggen's own
  windows by cooperative rect reporting, show-while-any-app-frontmost, jump out
  of the tray icon. Today each shell opens its own pet window. No code.
- **Locomotion in the web overlay** (ledges marked by the UI). No code.
- **Roll digest** — a short episodic digest written when a thread rolls. Not
  built; the `Previously:` seed and per-turn capture stand in.
- Original VRM model to replace the dev placeholder.
- **Batch-of-runs-finished** herald (working→idle count reaches zero, background
  only).
- **Phone push carrying a wait's approve/deny** (the phone reads the pending list
  today).

## Where things live

| Concern | Location |
|---|---|
| Agent definition | `agents/yinyue.md`; places `agents/places/` |
| Pet window, tray | `linggen-app/shell/src/pet.rs`, `menubar.rs`, `companion.rs` |
| Renderer | `ui/` — `PetStage`, `?pet=1`, `YinyueAvatar`, `useYinyuePresenter` |
| Surface registry, speak cue | `src/server/state.rs`, `src/server/api/yinyue.rs` |
| Voice / hearing | `src/server/api/tts*.rs`, `stt*.rs` (`stt_apple.rs` + `apple_speech/` on a Mac), `mlx_sidecar.rs`; models `runtime.rs` |
| Her turns, rolling session | `src/server/resident/` |
| Config | `[pet]` in `crates/linggen-engine/src/config.rs` |
| Watch loop, herald, faults | `src/server/resident/` (`triggers.rs`, `herald.rs`, `mission_fact.rs`, `run_fault.rs`) |
| Ambient, look-again | `resident/ambient.rs`, `resident/look_again.rs` |
| Moments | `src/server/yinyue_moments.rs` |
| Speak / listen doors | `src/server/api/yinyue.rs`, `tts.rs`, `stt.rs`, `live.rs` |
| Notes | `crates/linggen-engine/src/notes/` |
| Tools | `engine/tools/builtin/` (`agent_chat.rs`, `pet.rs`, `sense.rs`) |
| Her places per surface | `agents/places/` |
