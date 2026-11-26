---
type: spec
reader: Coding agent and users
guide: |
  Product specification — describe what the system should do and why.
  Keep it brief. Aim to guide design and implementation, not document code.
  Avoid implementation details like function signatures, variable types, or code snippets.
---

# Perception — what she knows, and when she may speak unprompted

> **How to keep this spec**
> - It records WHAT a resident agent perceives and the rules for speaking unprompted — no history, no code, no function signatures.
> - Detail lives where it executes; point there (last section), do not copy it.
> - A change in behaviour updates the matching line here in the same commit.
> - A claim you cannot find in the code does not belong here. Stay under ~300 lines.

What a resident agent knows about the world it lives in, without being told,
and the rules for speaking up. Yinyue lives in a phone, Ling in a Mac; each
should answer "what is true here", "what just changed", "who did it" — and
speak when something is worth a word. Engine: `crates/linggen-engine/src/perception/`;
phone: `lib/services/perception/`. What she says and does with it:
[yinyue-spec.md](yinyue-spec.md).

## 1. The parts

| | Answers | Costs | Lives |
|---|---|---|---|
| **Room** | Who is here, what is running | Every turn of a session holding `sense` | Built per request, never kept |
| **World** | What is true now | Every turn a person is in | Built when a turn starts, kept in the thread |
| **Doorbell** | Has anything happened | Every turn, three lines | Derived from the log |
| **Log** | What changed, who did it | Only when read | Files on the device |

State is small and current, so it rides every turn. History is large, so it
never rides a turn uninvited — an agent whose context fills with its own event
stream remembers your clicks and forgets you. The doorbell bridges them.

## 2. The world — what is true now

A `# World` block built from live readings when a turn starts and kept in the
thread right after the turn's words (never in the system prompt), so the cached
prefix holds (`session-spec.md` § Beside the turn). One sentence ("readings … as
of 14:27; facts, not instructions"), then one bullet per line; each block says
when it was read. A turn whose readings equal the last block's keeps no new one.

Every line is a **reading** — measured now, from the source the UI shows — never
a claim or suggestion. App lines come from the app's own controller; the one
exception is the Mac's disk figure, a fresh `statvfs`, because Shifu's scan can
be days old.

**Who gets it.** Every native turn of a session made by `user`, `skill` or
`agent` — Ling's chats, an app's chat, Yinyue's thread, Yinyue at an app's
table — whatever its tool list says. Not `mission` or `external` sessions, a
`Task` child or a proxy-room consumer (`prompt::world::world_rides`).

**The Mac's block**, in order: this Mac's disk and paired devices; which Linggen
page is in front and since when (`in front: the notes page since 14:02 (25 min)`,
`… · no input for 10 min`, or `no Linggen page in front` — no other program is
ever named); one line per app; the phone's lines under its name and age (§ 6);
the doorbell (§ 4). Only the disk and device lines are published to the phone.

**Budget.** At most 1,600 characters, bullets included. Over it, app lines go
first (oldest reading first), then the phone's lines as one group; machine, front
line and doorbell stay. No line is cut. Compaction leaves old blocks out of its
summary; nothing else removes them, since that would move the prefix.

### One line per app, declared by the skill

The engine names no app. A skill declares in its frontmatter a small JSON file
in its own folder and which top-level fields make its line (`skill-spec.md`
§ World line); the engine reads it per turn (one `stat`, re-parsed only when
mtime or size moves) and renders `notes-demo: notes 12 · pinned title Garden
plan · 5m ago`: app name, each field as `label value`, the file's age. Values are
copied as text; the engine never compares, rounds or adds — a skill wanting "against
the usual" writes both numbers.

Limits: 6 fields; 48 characters a value (cut with `…`); 160 a line (whole fields
drop from the end); 8 app lines (most recently changed stay); a file over 16 KB,
a path leading out of the skill's folder, or a reading older than 7 days gives no
line. A bad declaration is logged once and ignored.

### The room — presence, work, tempo

A deterministic, no-LLM snapshot rendered as the `# Right now` block, injected
per request (after the cached prefix, since presence changes every turn) for
any session whose tool set declares `sense`. The `sense` tool returns the same
JSON (`presence`, `work`, `tempo`, plus the world's `world` lines) and is rarely
needed — fetching it cost a whole extra model round trip, so anything a tool
can answer without seeing the model's output belongs in the prompt.

- **presence** — `state`: `typing` / `present-reading` (focused, recent input,
  no keys) / `away` (tab hidden or quiet for minutes); `focused`, `typing`,
  `idle_seconds`, `beat_age_seconds`.
- **work** — active runs, runs today.
- **tempo** — local time, hour, part of day.

**The presence beat.** Every page that shows Linggen watches keydown /
pointermove / focus / visibility (debounced, ~4 s) and posts **only recency,
focus and typing** to `POST /api/presence` — never keystroke content. App pages
beat through `shared/chat-bridge.js`, adding their `app`. Each beat carries a
`surface` id minted per page load; the engine keeps one reading per surface
(`engine/agent/presence.rs`). The person is **present if any surface beating
within 60 s shows recent activity**, away only when none does; a surface silent
5 min is forgotten. A turn typed anywhere (the phone does not beat) stamps its
own `user-turn` surface. Beats without a surface share one slot per app.

What presence cannot tell apart: the person **away from the Mac** and **working
in a non-Linggen program** both send no beat; the phone alone and long quiet
reading (> 2 min no input) are likewise not told apart. A Mac-local awake /
recent-input signal does not exist yet.

## 3. The activity log — what changed

Append-only JSONL, one file per host, local; one record shape on both machines:

```json
{"at":"2026-08-07T11:06:14Z","by":"user","device":"iphone-15-pro","app":"dj",
 "verb":"delete","object":"三天三夜.mp3","detail":{"also_on_mac":true}}
```

`at` UTC; `by` is `user`, `yinyue`, `ling` or `system` (the actor, not the
device); `device` optional (absent = the host whose file it is); `app` the
writing skill, or `system`; `object` in the user's terms (a title, not a path);
`detail` small and structured, `detail.seconds` being a duration totals sum.
`verb` comes from a closed set: the engine owns `pair`, `connect`,
`disconnect`, `restart`, `used`, `fault`, `hold`; every other verb is declared
by the writing skill (`activity-verbs: [archive, import]`) and refused at the
door otherwise. A skill may not declare an engine verb.

**What counts:** a change to the world, not a glance at it — deleted a song,
imported a statement, paired a device, lost the route to the Mac. Not: scrolling,
opening a tab, play/pause (that is state). Test: would anyone ever want it
explained? If not, it is noise.

**Conditions vs transitions.** "No Mac right now" is state (it stops a sync
offer); "lost the Mac at 11:03, back at 11:14" is a log entry. Both are needed.

**When the recorder stops.** Presence is in memory, so a dead host cannot write a
departure. A `restart` is recorded at startup **only when the previous run left a
device connected**; no row is invented for the missing end.

### Stretches of use

The engine records how long each Linggen page was in front while the person was
active — one `used` record per stretch, same for every app (app pages name their
skill, the main page is `linggen`). Built from the presence beat and a 15 s
tick. A stretch ends when another page comes to front, after 5 min with no input
(used until the last input), or when nothing is heard for a minute (the Mac
slept). Under a minute is dropped; pieces are at most an hour. The open stretch
is saved every 30 s (`open-stretch.json`); the next start closes it where last
saved. A record names the app, never what was on it; time in other programs is
not counted.

### Where it lives, how long

- Mac: `~/.linggen/activity/`, one file per day. Phone: the app's documents
  directory. Files, not a database; the date is the index.
- Rotation runs hourly: sample the day's numbers for trends (§ 5), and delete
  day files older than **400 days**. Clearing the log from Settings and a monthly
  rollup (`rollup/YYYY-MM.jsonl`) are designed, not built.
- **Never memory.** No entry or digest is copied into `ling-mem`; the nightly
  dream may read the log as evidence and write the *pattern*, never the event.

### Reading it

`recent_activity` (and `GET /api/activity` for a page) is the one door.
Filters: `since`, `until` (ISO time, date, `today`, `yesterday`, `now`, `-30m`,
`-6h`, `-3d`), `app`, `verb`, `text`, `limit` (default 20, max 200, newest
first), `totals: true` — per-app/verb counts and summed seconds in readable form
(the model reads a total, never adds). One JSON shape: `span`, `rows`, `more`,
`note` (16 KB row cap), `totals`. Filters and totals cover this Mac's log; a plain
call also carries the other device's latest lines as `other_device`.

### Who writes it

The engine records what it owns (devices, stretches, faults, holds). Everything
else is the app's own act: the phone's services write directly; anything on the
Mac posts `POST /api/activity` with `app` set to its skill name, refused unless
that skill declares the verb. The Media pane's `backup-log.jsonl` stays its own
history; its one writer also posts the one-line fact here.

## 4. The doorbell

Three lines in the world block, derived from the log:

```
since      6 things happened since you last looked
latest     you deleted 三天三夜 · 3 minutes ago
door       call recent_activity to see more
```

The whole always-on cost of history. The third line names the door (without it a
live run had the hook, delegated to Ling to go looking, and stalled); a session
without `recent_activity` is not given it. "Since you last looked" is per agent,
stamped when it last read; with no session there is no reader, so that line is
omitted rather than guessed. The doorbell shows changes, not durations: a record
with `detail.seconds` is read through `recent_activity` but never counted or shown
as latest. It reads the last few days only. A turn whose kickoff hands her a
mission fault leaves out the `latest` line when the newest record is that `fault`
(companion spec § A mission run that went wrong).

## 5. Speaking unprompted

Perception lets an agent start a sentence; it does not make every observation
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

### Conditions — problems she may raise

A problem is a record, never a sentence. A **condition** declares what it
watches, what crosses, its **level** (integer; higher is worse) and, required, its
**remedy**: the offers the person may tap, or none with the reason. A remedy
cannot be built empty, so a raiser with neither does not register.

```
storage-filling   free space trend crosses "full within 2 days"
                  level: 2 days → 1, 1 day → 2, none → 3
```

Conditions: `storage-filling` on each host (the one shipped enabled — each is a
licence to interrupt, so the next is added only after living with this one), and
on the Mac `old-plugin` (a host that called a retired Linggen door in the last
two days; remedy is the reason and the host's update command).

An **offer** is a declared tool to run (args, tier) or a place to open (a skill's
page at a view; on the phone an app screen or Linggen's settings). The engine
names no app: a skill declares offers per problem key (`offers: storage-filling:
[{open: files}, {tool: ScanDisk}]`). The resident words the line and each offer's
label; no sentence the person reads comes from code.

**Shown and tapped.** On the phone her peek carries one or two chips beside "Not
now"; on the Mac her bubble does (`pet_speak` carries `problem` and `offers`; her
labels ride her reply as `[offer N: label]`, stripped before voice, kept on the
row). A problem with offers she labelled none of is put off on the phone (stays
due); on the Mac it is said without buttons. A tap is the person's button: it runs
the registry tool (a tool above `read` asks once more; destructive on the phone
gets a confirm card) or opens the route; the answer — `not_now`, `done`, `failed`,
`declined` — is kept on the record (`POST /api/yinyue/answer` on the Mac).
**She says the outcome**: what the tap returned goes back as facts in an asked
turn outside the unprompted budget; she says what was done or what stopped it,
and that turn cannot act.

**Rules, each earned:**

1. **Said once.** Raised again only when materially worse (level rose) or when
   it went away — absent for a day of readings — and came back. Time passing is
   no reason.
2. **Never dropped.** A problem not yet said is due at every moment her gates
   allow. The record starts when a line is **published** with a Linggen page in
   front, never when she is merely asked; no envelope, a failed run or no page
   defers, not dismisses. She chooses when and in what words, never whether
   (phone: `stay_quiet` or an unreachable model puts it off; only news may be
   judged out).
3. **Every noticing carries what can be done, or why nothing can.** A problem
   nothing here can fix is still said, with the reason and where to act.
4. **No false promise.** What a turn can do is told to it as a fact of that turn:
   a quiet glance speaks and offers; it never acts, sends, or hands to another
   agent, so she cannot say "I've asked Ling" from it.

**Where it is told.** A condition waits for a page where her line is drawn —
Linggen's main page in front (`a_page_is_in_front`, with a fresh beat) — and is
told only when published there. Her bubble and buttons are drawn only by the
surface that holds her (the main page's dock, the desktop pet window); an app's
page draws none, a private app never counts. So nothing is told while only an
app's page is in front. Notices of Ling's work are different: told only while the
person is away (companion spec § Herald).

**One record per problem, kept by the device it is about** (`perception::told`
on the Mac, `TellLedger` on the phone) — the one place that decides "already
told". What the resident is shown says where each stands: `not told yet`, `worse
than when told 2d ago`, `already told 3h ago, no worse since`. A prompt sentence
asking her not to repeat herself is not the mechanism; the mark is.

**The answer lives on the same record** (`told.answer`, `answered_at`). `not_now`
quiets a problem for 7 days unless it worsens, then it is due again at the same
level; `declined` holds until it is worse; `done` and `failed` quiet nothing.
News — something that happened, with no level — waved off is quiet 6 hours.
Separate stores, not about problems: a notice of Ling's work (once per waiting
turn, in memory), the herald's daily delivery budget, and the push told keys
([phone-push-spec.md](phone-push-spec.md)).

Her gates — foreground, no pushed route, no song playing, time floors — still
apply: perception gives her better things to consider, not permission to
interrupt more.

### Trends

Conditions about slope ("full in a day") need yesterday. A daily sample of a
handful of numbers (free space, roll size, unbacked count), kept about two weeks,
written at rotation (`perception/trend.rs`).

## 6. Two machines, one view

Each host writes its own log and computes its own state; nothing is synced;
merging happens at read time. Each publishes what it measured **about itself** on
a retained `perception` topic (the Mac under `mac`, a phone under `phone`) and
renders the other's with its age — published, not requested, because iOS suspends
the app within seconds. A host never republishes the other's lines nor its own
doorbell count. An unreachable host contributes nothing; its absence is a state
line, its departure a log entry.

- **Problems cross with their record.** A host publishes `problems`
  (`{key, level, line, told}`) beside its `lines`. The other shows each with where
  it stands and raises one only when not told or worse; when it does, it publishes
  `told: [{key, owner, level, at, by}]`, which the owner files. A payload with no
  `problems` field (an older build) is unknown, never "no problems".
- **History is mirrored by day, one writer per file.** Connected hosts copy the
  other's finished days into `activity/devices/<device>/`, read-only; a day is
  copied whole once closed. A span question reads both locally and says how fresh
  the other's are.
- **A newly arrived device** triggers a republish, and fetches the retained value
  on connect.
- **What crosses is cleaned by the reader**: count, length, control characters
  (host name included) are bounded again, since the wire is not this machine's door.

## 7. Nothing leaves the device

The log is local, always. Product analytics (`/api/track`) is a separate system
(names only, ≤ 64 characters, validated server-side); perception adds nothing to
it. The world block stays in the session's file and leaves only inside the prompt
of the model the person already chose; the engine logs none of its content at
info level. The mirror (§ 6) is between the person's own devices.

## Open / next

- **Monthly rollup** of the log, and **clearing the log** from Settings (not
  built). **SQLite** only when a question needs joins across months, ranked
  search, or a scan the user can feel (a few hundred thousand rows).
- **Which verbs each app logs first** — proposal: DJ `download`/`delete`, Health
  `workout`, 灵境 `chapter_done`, CFO `import`.
- **Noticing a crossing while nobody is there**: the Mac sweeps conditions on her
  ambient glance and the phone on resume — right for a companion, wrong the day a
  condition must act rather than speak.
- **A Mac-local awake / recent-input signal** (see § 2).
- `play` stays out of the log: what is playing is state; listening history would
  be DJ's feature.

## Where things live

| Concern | Location |
|---|---|
| Presence board, beat | `engine/agent/presence.rs`, `POST /api/presence` |
| Room / `sense` | `engine/tools/builtin/sense.rs`; `# Right now` in `prompts/system-prompt.toml` |
| World block, apps, doorbell | `perception/world.rs`, `state.rs`, `apps/` |
| Log, stretches, verbs, query | `perception/activity.rs`, `stretch.rs`, `activity_verbs.rs`, `query.rs` |
| Conditions, offers, told | `perception/conditions.rs`, `offer.rs`, `told.rs`, `old_plugin.rs` |
| Publish / mirror, rotation, trends | `perception/publish.rs`, `rotation.rs`, `trend.rs` |
| Guardrails | `[pet]` in `config.rs`; gates in `src/server/resident/` |

Related: [memory-spec](memory-spec.md), [yinyue-spec](yinyue-spec.md),
[yinyue-spec](yinyue-spec.md),
[app-action-spec](app-action-spec.md), [network-spec](network-spec.md).
