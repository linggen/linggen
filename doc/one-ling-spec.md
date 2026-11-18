---
type: spec
reader: Coding agent and users
status: designed 2026-10-08; the daily session built behind a setting 2026-11-16 (2.4 branch)
guide: |
  Product specification — what the system should do and why. Brief; guides
  design and implementation, not a code walk-through.
---

# One Ling, one Yinyue

There is one Ling and one Yinyue, on every device, like real people. A device
is where a turn runs, not who answers. The Mac and the phone have the same
shape: two agents at one table, Ling answering. Settled with Hanli 2026-10-08.

## Related docs

- `shared-session-spec.md`: the table — one thread, members, context per member.
- `app-action-spec.md`: tools, tiers, typed cross-device calls.
- `yinyue-spec.md`, `yinyue-companion-spec.md`: her body, heralds, budget.
- `webrtc-spec.md`: the phone ↔ Mac transport (WebRTC only).

## Roles

- **Ling** — the keeper of the Linggen engine. Does every task, runs the apps,
  owns performance, safety and critical jobs, on whichever device has the
  tools. The Ling on the Mac and the Ling on the phone are the same Ling.
- **Yinyue** — the user's companion: 银月, the proud fox 妖王 of the story
  (`agents/yinyue.md`). Greets, notices, steadies them in her own haughty way;
  speaks on her own moments. She runs no tasks and never
  hands work across devices.

One persona source for each. The phone's Yinyue already loads her soul from
the engine; Ling's persona follows the same path, so the two devices never
drift.

## Who answers

One rule, the same on every device. No classifier.

- **Ling answers every user message**, unless it is addressed `@Yinyue`.
- **The input box pre-fills `@Yinyue`** after the user addresses her and after
  each of her replies — visible, deletable. Deleting it hands the next line
  back to Ling. It clears after about 10 minutes of quiet.
- **Forwarding.** Either agent may pass a message to the other at the same
  table (Ling hands "rough day" to Yinyue; she hands a task to Ling). One hop:
  whoever receives a forward answers it and cannot forward again. The thread
  shows a thin `Ling → Yinyue` line.
- **Yinyue is proactive.** Notifications, greetings, warm-ups and things she
  noticed, under her herald budget and her SILENT contract.
- **Skill moments** (a story beat in Lingjing) may give her a turn after
  Ling's; the skill decides, not the dispatch.

One speaker per turn, so a message costs about one model call.

## Where a turn runs

- **Mac work goes to the Mac app's latest session.** When a task needs the
  Mac, Ling on the phone hands it to the relevant app's most recent session on
  the Mac (the lookup already built for DJ), and Ling continues it there with
  the Mac's model and tools. The user sees the work in that Mac session; the
  phone shows it as Ling's own step (`Mac · DJ`, live status) and Ling reports
  the result in her own words. It is one Ling working on two devices: the
  hand-off is written as Ling continuing her own task, never as Yinyue (or
  another agent) asking her.
- **Mac unreachable → Ling says so and stops.** No queue, no waking the Mac.
- **Tools run where they live.** Mac tools on the Mac, phone tools on the
  phone, reached as typed calls (`app-action-spec.md`). Each tool chip shows
  where it ran: `Mac · dj.download`, `iPhone · save to Photos`.

Example: on the phone, "download this song to my phone". Ling hands it to
DJ's latest session on the Mac, continues there (download, add to the phone
view), and on the phone says it's on its way. One Ling, two devices.

## No agent-to-agent hops across devices

Cross-device work is a reach, never a conversation. Today `ask_mac_app` has
the phone's Yinyue write to the Mac app's Ling, who answers her — two agents
chatting. The same path stays (latest app session, visible on the Mac), but
the sender becomes Ling herself continuing her task, and exact-param actions
become typed tool calls with no second turn at all.

## Sessions stay on their device

- No session sync between devices. A session lives where it was started.
- The phone lists the Mac's sessions (title, last line, time) and streams one
  in when opened; recently opened ones stay cached.
- **One memory, not one thread.** What Ling and Yinyue know across devices
  comes from memory (ling-mem; the phone's notes drain to it) and the world
  state — a tool that reads what each device is doing and has done (retained
  topics, the activity log). They say how fresh a remote fact is ("as of two
  hours ago on your Mac").
- Shared data (DJ playlists, CFO, the Lingjing save) keeps its own per-app
  sync lanes and single-writer rules.

## The phone

- **Home is the Chat.** Ling answers there; Yinyue lives in it — her greeting,
  her cards, a tap on her face to address her. This replaces "Yinyue's page is
  the home" (2026-09-10).
- The two chat screens (Yinyue's, and the Mac's Ling) become one **Chat**. The
  Mac stops being a separate conversation and becomes a place Ling can reach.
- The phone runs a small two-member table in its Dart loop: Ling and Yinyue,
  context built per member as in `shared-session-spec.md` § The thread.
- **Memory writers:** agents write, apps never (replaces "only Yinyue writes
  phone memory").
- **One thread on the phone, as today.** The Chat keeps a single continuous
  thread (`yinyue/thread.json`: resumed within 24 h, else fresh; older turns
  folded into its summary), now holding Ling's and Yinyue's lines. Simple, and
  nothing piles up. A 90-day retention rule and a Settings clear button come
  only if the phone ever keeps many sessions.

## Linggen's daily session (2.4, behind a setting)

`agent.daily_session` (Settings → Session), off by default. An explicit
On or Off is saved; no choice means the engine's default. Off, every
session, prompt and row is as before. On (decided by the builder 2026-11-15/16):

- **The day's session** — `sess-day-YYYY-MM-DD`, the day beginning at 04:00
  local; made on first use, found again by its id after a restart; Ling its
  lead, Yinyue seated when she first speaks there. It does not roll to
  segments; a full thread compacts as a shared session does.
- **What joins it**: an app's chat (a page asking for its app's session),
  Yinyue's own and unprompted turns, a hand-off from the phone. **What stays
  apart**: missions, delegated runs (`agent_run`), a New chat the person
  makes, an app's one-off sessions — one bound to its own values (a test
  save) or made with tools withheld (an NPC's talk, a prefetch) — and a
  private app's chat.
- **One rule into the day.** Every way in — a new chat, a chat that names a
  day, an app's list, the day's lookup (`GET /api/sessions/day?skill=`), her
  moments, a message delivered to an agent — asks one rule: home's chat
  joins; an app's only when it resolves and does not declare
  `private-talk: true`. An app that does not resolve stays apart.
- **Each turn in its place** (`session-spec.md` § A turn's place): the page's
  app gives the turn its skill, tools, folder and grants; one turn at a
  time, a message from another app waiting for the running one.
- **A private app** (`private-talk: true`, `skill-spec.md` § Private talk)
  keeps a session of its own, as every app's chat does with the setting
  off, so the day holds none of its talk. A chat that names the day from its page stands in the app's own
  chat instead, and so does a person's `/<app> …` (or the app's trigger)
  typed in the day. In a day it never becomes a turn's place nor its
  active skill: the Skill tool is refused it with a reason the model
  reads. From its turns — a subagent's and a turn the engine starts
  there included — a tool that carries words out is refused; a question
  waiting there reaches Yinyue only as that the app waits. Its world line
  stays. The day's session is made by `agent`, so the memory scan (which
  reads a person's own chats) never reads it.
- **Her moments** run in the day's session too, standing in the app they
  came from; her line is a row there and lands nowhere else. A private
  app's moment runs at the app chat it names, never in a day; with none
  named it goes unanswered.
- **The list** marks the day's session (`day: true` on its row), one row a day; a page folds each
  stretch from another place to a line (the app, how many messages), which
  opens on a tap. The chats from before stay listed and readable either way.
- **An app's own page** (`GET /api/skill-sessions?skill=`) lists the day's
  first, marked the same way, then the app's other sessions — its chats
  from before the day and its one-off ones — each with its own members and
  model. The page picks up the day's, spoken in yet or not
  (`shared/api.js` `pickResumable`): a new message lands there, and an
  earlier chat is still there to open.
- **The phone** (engine side): a hand-off takes the first of
  `GET /api/skill-sessions?skill=` (the day's, for an app that joins it) or
  `GET /api/sessions/day` for the Mac's own chat, and names its app as
  `skill_name`; a queued hand-off keeps its `client_id` on its row when it
  runs; open notes read back from `GET /api/notes?agent=`, read-only. Its
  own thread is unchanged.
- **A day per person**: a person other than the builder has a day of their
  own (`sess-day-YYYY-MM-DD-p-<person's id in hex>`). The person behind a
  caller is resolved once, from the device it named, as memory resolves
  it: the owner's own device is the builder; a phone signed in to another
  account is that account, a signed-out phone its device — on the channel
  and on plain HTTP from the LAN alike. A peer that has not said which
  device it is is no one and is given no day. A day is read, written and
  shown (the page state of a page viewing it) only by its own person: from
  off this Mac, any request naming another person's day (its question, a
  plan, its grants, a stop, a delete, its pictures, its prompt) is refused
  with 403. An agent's notes and her own thread are the builder's (a
  signed-out phone is its own person, as memory reads it); what the engine
  starts itself (her moments) is the builder's, and a question waiting in
  another person's day is not heralded in the builder's thread or voice.
  Her stage is held per person: a line she says in a person's day or
  session reaches the newest stage that person's own devices hold (none:
  nowhere), what the builder says to her goes to the builder's stage, and a
  stage names only a session its person may enter. An app moment is its
  sender's: another person's is not taken up. Presence, kept topic
  readings and phone facts are the builder's devices'; another person's are
  dropped. A day is made once and never written over.
- **Clear** takes the rows of the place it is pressed in: an app's page its
  app's, the main page home's. The compaction summary is the day's, no
  place's: no Clear takes it and no page folds it. Clear in the day cancels
  no run (the one running may be another place's); Stop stops whatever runs.
- **Whose work a page shows**: the engine names the place on what it sends
  (`turn_place::told_place`, a day's session only — nothing names one
  anywhere else, and a private app is never one). While the running turn
  stands in another place, the status line and Stop name it ("Working in
  Health", "Stop · Health"); Stop still stops it. The running top-level
  run on the page state and its status events carry `place`. A waiting
  message carries the place it was sent from (`place` on the queued item):
  another page shows it as "Waiting from Music", and only its own page
  offers Edit, Remove and Steer. What a running turn streams (its tokens,
  text, blocks, tool output, messages — the person's that started it too)
  carries its place, so another place's turn folds while it streams, as its
  saved stretch does: one line ("Health · working…") that opens on a tap.
- **An app's own scripts hear only their own place**: the chat relays a
  run's stream, blocks, end and signs of life to the app's page
  (`/shared/chat-bridge.js` handlers) only when the run stands in that
  page's place, as the engine first told it; another place's turn stays in
  the chat, folded. With no place told (setting off, any other session)
  every run is relayed as before.
- **A question names where it was asked**: a question card shows at every
  page of the day and is answered from any of them (it is the person's to
  answer), and when the asking turn stands in another place the card says
  so ("From Health"). The engine tells the place on the `ask_user` event and
  on each open question it lists (`pending_ask_user`, `GET
  /api/pending-ask-user`); the page never works it out.
- **One model a member for the day**: only the picker keeps one
  (`PATCH /api/sessions`); a model a page sends with a message is for that
  turn and is not kept.
- **Compaction settings**: a page's `POST /api/chat/compact_config` sets
  nothing in the day (it answers success); the day compacts at the global
  threshold with no focus.
- **A page removes only its own app's sessions** (`DELETE
  /api/skill-sessions`, setting on or off): the session's own skill must be
  the one the page names; with none named, or another's, it is refused (403)
  and nothing is removed. The day's session is every app's and is refused.
- **A `!` command** stands where a turn from the same page stands (its page
  names its app as a chat turn does): at an app's page it starts in the
  app's folder, and its `cd` is kept for that place alone; home's starts in
  the day's own folder. A private app's page gets home's workspace.
- **Copy system prompt** shows the prompt and tools of the place it is
  pressed in (the page names its app): an app's page its app's place, the
  main page home's. A private app's page shows none of the app.
- **Each member is itself at home**: at home in the day a member other
  than the lead (Yinyue) is offered its own tools, not the lead's set — so
  her talk there reads the room (Right now) as her own thread does. At an
  app's place the place says what each member is offered; a shared session
  a person made keeps the table's set (`shared-session-spec.md`).

Built (2.4 branch, 2026-11-16/17): all of the above, in `server::day_session`, `server::person`,
`state_fs::day_session`, `TurnPlace::private_app` and `tools::exit_gate`.

Not built: the phone's side (it must send `skill_name` and use the day's
id); a light turn after her moment stands at home, not in the moment's app;
a page left open past 04:00 keeps yesterday's session; recall of the day's
thread beyond the folded view, a summary cache.

## Tool names and packs

- **Every app tool is named `<app>_<verb>`**: `dj_play`, `dj_delete_playlist`,
  `cfo_read`, `photos_find`. Underscore, not dot — model function names and
  retained-topic ops reject dots; chips may show `dj.play`. Mac tools reaching
  the phone (`mac/tools`) follow the same rule.
- **One tool per verb across devices** (Hanli 2026-10-08). When a Mac tool is
  the same verb as a phone tool of that app (an explicit per-app table, never
  a name guess), the phone offers one tool with `where: "phone" | "mac"` —
  phone = this phone's own copy, mac = the Mac's — and `where` appears only
  while the Mac is reachable. A Mac tool never replaces a phone tool; Mac
  tools with no phone counterpart are their own entries in the app's pack.
- **Packs stay the isolation.** Each turn carries only the active apps' tools,
  plus Ling's own and `common` ones; `open_toolbox` is the index (one line per
  tool of an app) and loads a pack on demand. No session switching.
- **Pack signals are deterministic**: the app screen the user was last on
  before opening the Chat (within a few minutes), DJ playing, apps used
  recently in the thread, a body word for Health. Today's "open tab" signal
  never fires, because typing happens on the chat tab itself.

## What changes

1. **Tool names + typed Mac tools for the phone.** Rename every phone tool to
   `<app>_<verb>`; replace the open-tab pack signal with the last app screen.
   The Mac publishes its app actions as a
   retained `mac/tools` catalog, as the phone does with `phone/tools`; Ling's
   phone loop calls them directly, with no second turn (DJ download first).
2. **Ling on the phone.** A Ling member in the phone's Chat; dispatch as in
   § Who answers; Yinyue's soul loses its task-running parts; the two chat
   screens merge.
3. **One Ling across the hand-off.** The phone-to-Mac hand-off (latest app
   session) is sent and framed as Ling continuing her own task; the phone
   shows it as her step and her result.
4. **Lingjing on the phone** (`skills/lingjing/doc/phone-prototype.html`):
   stage-first native UI, a pure-Dart port of the rules held to shared
   fixtures (as CFO's engine), Ling and Yinyue at the game's table, the save
   synced through the cloud save. Playable without the Mac.
