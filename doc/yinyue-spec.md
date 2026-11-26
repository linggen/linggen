---
type: spec
reader: Coding agent and users
guide: |
  Product specification — describe what the system should do and why.
  Keep it brief. Aim to guide design and implementation, not document code.
  Avoid implementation details like function signatures, variable types, or code snippets.
---

# Yinyue — who she is and where she appears

> **How to keep this spec**
> - It records WHAT Yinyue is, where she shows up and the rules each surface obeys — no history, no code, no function signatures.
> - Detail lives where it executes; point there (last section), do not copy it.
> - A change in behaviour updates the matching line here in the same commit.
> - A claim you cannot find in the code does not belong here. Stay under ~200 lines.

Three docs, each owning one thing: **this one** — who she is and where she
appears; [yinyue-companion-spec.md](yinyue-companion-spec.md) — what she says
and does; [perception-spec.md](perception-spec.md) — what she knows and when
she may speak unprompted.

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
  muted, or at the page the line shows on (companion spec § Heard or read), the
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

1. **Qwen3-TTS** on the GPU (MLX, in the shared voice sidecar), speaker from
   `[pet] voice`. "Natural" is exactly this voice being on disk.
2. **Kokoro-82M** (candle, `any-tts`; the `kokoro` build feature, default on).
   Its voice id carries the language: Han characters pick Mandarin, else English.
3. **macOS `say`**, the system voice.

`GET /api/tts` says whether she can speak and whether naturally, starting
nothing; `POST /api/tts {text, voice?}` returns a WAV and fetches no model in
front of a line. Hearing (STT) is Whisper large-v3 turbo in the same sidecar
(companion spec § Asked aloud).

A line is spoken in short clauses (`api/tts_text.rs`): the cue carries them,
the surface fetches each and plays the first while the next is made. The bubble
shows the original text; only the spoken copy drops quotes and a leading
「……」 and turns 「——」/「……」 into pauses. Whether a line is spoken at all is
companion spec § Heard or read.

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

## Where things live

| Concern | Location |
|---|---|
| Agent definition | `agents/yinyue.md`; places `agents/places/` |
| Pet window, tray | `linggen-app/shell/src/pet.rs`, `menubar.rs`, `companion.rs` |
| Renderer | `ui/` — `PetStage`, `?pet=1`, `YinyueAvatar`, `useYinyuePresenter` |
| Surface registry, speak cue | `src/server/state.rs`, `src/server/api/yinyue.rs` |
| Voice / hearing | `src/server/api/tts*.rs`, `stt*.rs`, `mlx_sidecar.rs`; models `runtime.rs` |
| Her turns, rolling session | `src/server/resident/` |
| Config | `[pet]` in `crates/linggen-engine/src/config.rs` |
