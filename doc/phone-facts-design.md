---
type: design
reader: Hanli (review before any build), then the coding agent
status: DRAFT for review, 2026-09-24. Nothing built.
---

# Phone facts (手机事实)

The phone does real-life chores the Mac never hears about, so quests can't count them. This design gives the phone one general way to say **"this kind of thing happened, at this time"**. The engine turns that into quest stamps, and it never learns what any app means by it.

Related: `perception-spec.md` (activity log, retained `perception`), `app-action-spec.md` (reads are published, actions are queued), `webrtc-spec.md`, `src/server/milestones.rs`, `src/server/quests_watch.rs`.

## Why it's missing today

- Phone to Mac has three lanes. None of them carries a fact that lands on the Mac:
  - **device topics** (`topic_publish`): a relay, sometimes retained. `from_device` is whatever the client claims.
  - **app op logs** drained over `http_request` on the data channel (DJ `phone-ops`, CFO LWW sync).
  - **ActionQueue**: Mac asks, phone does.
- The phone's activity log (`services/perception/activity_log.dart`) stays on the phone by design (perception §8).
- `milestones.rs` only sees `PhoneChat` when a chat rides the Mac's channel (`rtc/peer/control.rs:126`). Yinyue on the phone's own model never gets there.
- DJ's `track-played` op *is* queued (`dj_player.dart:132` → `dj_library.dart:162`), but `dj/scripts/actions.mjs` doesn't know it and skips it. The phone then treats it as a spent fact (`dj_sync.dart:52`).
- One precedent already works: Health's letters screen stamps `opened_at` on a synced letter, and `health/scripts/quest.mjs` derives `health-report` from it. **Rule:** where an app already syncs a record its quest writer can read, use that record. Phone facts are for moments that leave no synced trace, such as views, chats and PhotoKit deletes.

## 1. The fact

```json
{"type": "fact", "kind": "cfo-import", "at": "2026-09-24T14:03:11Z"}
```

- `kind`: `<app>-<verb>`, `[a-z0-9-]{1,48}`, from a closed Dart enum (`PhoneFactKind`). The dash matches the retained/op charset.
- `at`: when it happened on the phone, UTC. It is not the delivery time.
- **The device comes from the channel, never the payload.** The engine takes it from the peer's bound `identify` actor. A fact from an anonymous peer is dropped.
- Nothing else is sent: no count, title, amount, filename or text.

## 2. Transport: the latest value per kind, republished on connect

A quest's `done_at` only needs the **newest** time a thing happened. That makes it a *reading*, and readings want retention, not a queue (project_retained_topics).

- **Phone**: `PhoneFacts.note(kind)` writes `{kind: latest at}` to a small durable file in app documents. The file has one row per kind, so it is bounded no matter how long the phone is offline.
- **Sending**: if connected, the fact goes out at once. Every connect also resends the whole ledger. This uses the same hook as `ToolCatalogPublisher` (`tool_catalog_publisher.dart:33`). There are no acks and no outbox bookkeeping: ~10 tiny messages per connect.
- **Offline**: an offline phone just holds its ledger. The next connect, on LAN or over the relay, delivers it with the original `at`, so a Monday chore done offline still stamps Monday.
- **Dedupe (Mac)**: the key is `(device, kind, target)`, and the Mac advances it only when a *newer* `at` arrives and the target's stamp succeeded.
  - A failed stamp retries on the next resend.
  - A skill that declares a kind later gets the ledger's latest value on the next connect.
- The whole lane is WebRTC: a control-channel message. There is no HTTP route.

## 3. Who stamps: the engine dispatches, skills declare

The engine gets one general handler (`src/server/facts.rs`, fed by a `"fact"` arm in `rtc/peer/control.rs`). It keeps a ledger at `~/.linggen/facts/<device>.json` and sends each new fact to:

1. **Its own milestones.** `Milestone` gains an optional `fact:` field; for example, `linggen-phone-chat` gets `fact: "yinyue-chat"`. This file is the engine's own (`linggen.json`), so naming the kind there is legitimate.
2. **Skills that declare the kind** in `SKILL.md` frontmatter. The engine runs the skill's **own** quest writer, so each `<app>.json` keeps one writer:

```yaml
quests:
  stamp: bash scripts/quest.sh {id} {at}   # the one writer of quests/<app>.json
  facts:                                    # phone fact kind → quest id
    photos-clean: shifu-clear
```

- The engine fills in `{id}`/`{at}` only after validating them (id charset, strict ISO), then runs the command in the skill directory with a short timeout.
- The writer must **never move `done_at` backwards**. Shifu's `quest.sh <id> <when>` already guarantees this.
- CFO's `stampQuest` is page-only today, so it needs a CLI entry: `quest.js stamp <id> <at>`.
- The engine names no app. Would a second, unrelated skill use this? Yes: any skill with a phone surface.
- `quests_watch.rs` already announces `QuestsChanged`, so Lingjing refreshes on its own.

## 4. Facts to add now

| kind | Emit point (linggen-mobile/lib) | Stamps |
|---|---|---|
| `yinyue-chat` | `screens/chat/yinyue_chat_controller.dart:379`, in `send()` after the user entry. A person's typed turn only; never `wake()` or `retryLast()` | `linggen.json` `linggen-phone-chat` (engine table) |
| `cfo-import` | `services/cfo/cfo_repo_sync.dart:546` `_recordImport` (n > 0). Covers the button and the share sheet, since both go through `cfo_screen.dart:311` `_import`. Skip the dev `AUTO_IMPORT` (`sync:false`) | `cfo.json` `cfo-import` |
| `cfo-trends` | `screens/cfo/cfo_screen.dart:152` `_showSection(sectionTrends)`. Not from the dev auto-sections at `:127`/`:145` | `cfo.json` `cfo-review` |
| `cfo-invest` | same place, `sectionInvest` | new `cfo-invest` (week, pool) in CFO's menu |
| `dj-listen` | `services/dj/dj_player.dart:47`, where the queue reaches `completed` after a playlist ran through (≥ half its tracks counted by `_counter`) | new `dj-listen` (day, pool). **DJ has no quest file yet**, so it needs a `quest` writer plus a menu |
| `dj-karaoke` | `screens/dj/karaoke_screen.dart:84`: arm on open, emit on the first counted play (`dj_player.dart:131`) while the screen is up | new `dj-karaoke` (week) |
| `photos-clean` | `screens/photos/photos_controller.dart:743`, after `removed` is non-empty, which means past the OS confirm | `apple-shifu.json` `shifu-clear` |
| `health-doctor` | `screens/health/health_doctor_screen.dart:28` `initState`, once the note resolves non-null | new `health-doctor` (week, pool) in Health's menu |

Out of scope: `health-report` already works through letters. The `track-played` op should be fixed inside DJ (`actions.mjs` → play history). It is a separate bug, not a fact.

## 5. Privacy

- The wire and the Mac ledger hold only `kind` + `at` + the channel's device id: what, when, which device. A test checks this.
- Kinds are a closed enum on the phone, and the engine checks the charset. There is no free-text field anywhere, so content has no slot to travel in.
- The ledger stays local on the Mac. It never goes into telemetry (`/api/track`), the activity log or ling-mem, and it is never uploaded. Views are not activity (perception §3). Facts are quest evidence only.
- Quest files already carry only `done_at`, and they keep doing so.

## 6. Test plan

- **Engine unit**:
  - Rejects anonymous peers, a bad kind, a future `at` (> 5 min) or an `at` older than 30 days.
  - Monotonic dedupe per `(device, kind, target)`.
  - A failed stamp is retried on resend.
  - Frontmatter parse, and the command is built with quoted, validated args.
  - A milestone `fact:` stamps at the fact's `at`, not the tick.
- **Skill unit**: `quest.sh` / `quest.js stamp` / the new DJ writer never move `done_at` back and keep unknown entries.
- **Phone unit**:
  - The ledger coalesces per kind.
  - Nothing is sent while disconnected; the whole ledger is resent on connect.
  - The payload keys are exactly `type`, `kind` and `at`.
- **Live on 9527** (paired iPhone):
  - Airplane mode, chat Yinyue on the phone model, then reconnect. `linggen-phone-chat` `done_at` should equal the chat time.
  - Share a CSV into CFO. `cfo.json` `cfo-import` should be stamped, and the Lingjing page should update without a reload.
  - Delete one photo. `shifu-clear` should be stamped.
  - `grep` `~/.linggen/facts` and the device log: no titles, amounts or text.

## 7. Open questions for Hanli

1. **Wire: a dedicated `fact` control message, or `topic_publish` on a `facts` topic?** *Recommend a dedicated message.* The device comes from the bound identity instead of a claimed `from_device`. Nothing fans out to other surfaces. It gets one clear handler.
2. **Latest per kind (no acks), or a queued outbox with acks?** *Recommend latest per kind.* Every quest today is `done_at`-shaped, and this is bounded and self-healing. Add a counting outbox only when a quest needs "N times".
3. **Reuse quest ids or add new ones?** *Recommend reuse when it is the same chore on another screen:* cfo-trends → `cfo-review`, photos-clean → `shifu-clear`. Add new ids only for new acts: `cfo-invest`, `dj-listen`, `dj-karaoke`, `health-doctor`. Each skill's menu grows, and nothing is added to the engine.
4. **Does opening a view count?** *Recommend a 10-second dwell* for view facts (trends, investments, doctor). A tap-through then doesn't farm a quest, and a real read still counts.
5. **Keep the in-memory `PhoneChat` sighting?** *Recommend keeping it and adding `yinyue-chat`.* Both stamp `linggen-phone-chat`: one sees Mac-model chats, the other sees phone-model chats.
