---
type: spec
reader: Coding agent and users
status: designed 2026-10-08, not built
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
- **Yinyue** — the user's companion and emotional friend. Greets, warms up,
  notices, comforts; speaks on her own moments. She runs no tasks and never
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
- **Session files on the phone are kept 90 days.** Older ones are deleted on
  the phone automatically (checked at app start). Settings also has a clear
  button for chat history, with one confirm. Only chat session files are
  cleared: memory and app data (the Lingjing save, DJ, CFO) stay.

## What changes

1. **Typed Mac tools for the phone.** The Mac publishes its app actions as a
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
