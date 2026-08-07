# Perception Spec

**Status: built on both hosts** — engine (`src/perception/`) and phone
(`lib/services/perception/`). One condition ships enabled: storage filling.

What a resident agent knows about the world it lives in, without being told.

Yinyue lives in a phone. Ling lives in a Mac. Each should be able to answer
"what is true here", "what just changed", and "who did it" — and to speak up
when something is worth a word. Before this, neither could: they knew only the
conversation. That gap was not cosmetic. It is why Yinyue offered to sync songs
that had already arrived — nothing in her world said they had.

Perception is that world, in three parts that cost three different amounts.

---

## 1. The three parts

| | Answers | Costs | Lives |
|---|---|---|---|
| **State** | What is true now | Every turn | Recomputed, never stored |
| **The doorbell** | Has anything happened | Every turn, three lines | Derived from the log |
| **The log** | What changed, and who did it | Only when read | A file on the device |

The separation is the design. State is small and always current, so it can ride
every turn. History is large and grows, so it must never ride a turn uninvited —
an agent whose context fills with its own event stream remembers your clicks and
forgets you. The doorbell is what bridges them: it costs two lines and tells the
agent whether the log is worth opening.

---

## 2. State — what is true now

A block in the agent's prompt, beside the existing Environment block, rebuilt
each turn from live readings. Never accumulates, never persists, never stale.

Contents are per-host and per-app, and each line must be a **reading**, not a
claim: something measured now, from the same source the UI shows. A line no
consumer can act on does not belong here.

Phone, illustrative:

```
device      iPhone 15 Pro · 8% free (11.2 GB of 128) · battery 88%
mac         reachable · paired as "This-Mac"
music       43 songs · playing 上海灘 · auto-sync on · synced 4 min ago
photos      2,914 in the roll · 12 not backed up
```

Mac, the same idea about its own body: disks, the paired phones, what each app
holds.

**Source.** App lines come from the app's own controller. No new measurement
subsystem: perception reads what the app already knows, so the agent and the
screen can never disagree.

The one exception is the disk figure on the Mac, and it is the rule's own
doing: Shifu's readout (`data/readout.json`) is a *scan*, and a scan can be
days old. A line that stale is what §2 forbids, so perception takes its own
`statvfs` at the moment it is asked — the same number the Finder is showing.
The phone's `ReadoutPublisher` is live, so it is read as it stands.

**Consequence.** With state in hand, an agent stops asserting conditions it has
not checked. "Your Mac is asleep" becomes a reading rather than a guess.

---

## 3. The activity log — what changed

Append-only JSONL, one file per host, local. One record shape on both machines,
because two schemas for one idea fork on contact.

```json
{"at":"2026-08-07T11:06:14Z","by":"user","device":"iphone-15-pro","app":"dj",
 "verb":"delete","object":"三天三夜.mp3","detail":{"also_on_mac":true}}
```

- `at` — ISO 8601, UTC.
- `by` — `user`, `yinyue`, `ling`, or `system`. The actor, not the device.
- `device` — which machine this happened on. With `by`, this is the attribution
  the family/multi-user design already calls for; it is the same field, not a
  second one. Optional: a file is one host's, so an absent `device` means the
  host whose file it is.
- `app` — `dj`, `photos`, `cfo`, `shifu`, `system`.
- `verb` — a short closed-set slug: `delete`, `add`, `edit`, `sync`, `backup`,
  `clean`, `import`, `pair`, `connect`, `disconnect`, `restart`.
- `object` — what it happened to, in the user's terms. A song title, not a path.
- `detail` — optional, small, structured.

### What counts

An entry records **a change to the world**. Not a glance at it.

- Yes: deleted a song, edited a playlist, backed up 200 photos, imported a
  statement, freed 40 GB, paired a device, lost the route to the Mac.
- No: scrolled the library, opened a tab, played/paused (that is state — what is
  playing now — not history).

The test: would anyone ever want it explained to them? If not, it is noise, and
noise is what teaches a user to stop reading.

### When the recorder itself stops

Presence is held in memory, so a host that dies while a device is connected
cannot write that device's departure — the log is left with two `connect` rows
and nothing between them, which reads as a fault in the very record meant to
explain faults.

The answer is not to invent the missing row; nobody knows when the connection
actually ended. It is to say the one thing that *is* known, at the moment it
becomes known: this host started, and something was connected to the last one.
So a `restart` is recorded at startup **only when the previous run left a device
connected**. A restart nobody was attached to explains nothing, and a host may
restart many times a day — logging every one would bury the entries a person
actually wants to read.

### Conditions vs. transitions

A condition belongs in state; its **transition** belongs in the log. "No Mac
right now" is state, and it stops the agent offering a sync. "Lost the Mac at
11:03, back at 11:14" is a log entry, and it is what explains why something
failed while the user was away. Both are needed and they are not the same fact.

### Where it lives

- Phone: the app's documents directory, beside the app data it describes.
- Mac: `~/.linggen/activity/`, one file per day.

### Who writes it

The engine records what it owns: a device paired, a route lost or regained.
Everything else is the app's own act, so the app says so — the phone's services
call the log directly, and anything on the Mac with no way into the engine's
memory (skills, app pages, scripts) posts to `POST /api/activity`. The engine
names no app there; it takes the caller's word for what it is, exactly as
`/api/topic/publish` does.

That settles the Mac's `backup-log.jsonl`, which is the same idea in one lane:
it is neither migrated nor read in place. It stays the Media pane's detailed
history, and the one writer that appends it also posts the one-line fact here.
One writer, two readers — not one file two owners.

---

## 4. The doorbell

Three lines in the state block, derived from the log at build time:

```
since      6 things happened since you last looked
latest     you deleted 三天三夜 · 3 minutes ago
door       call recent_activity to see more
```

That is the whole always-on cost of history. It gives the agent the hook to
remark and, most of the time, the answer as well. When it needs more it calls
`recent_activity` — a tool, so it costs nothing until wanted, never enters the
window uninvited, and never renders in the thread. The third line names that
door, because a doorbell that does not say where the door is cost one live run
its answer: she had the hook, delegated to Ling to go looking, and stalled.

"Since you last looked" is per-agent, stamped when that agent last read. With
no session there is no reader, so the first line is omitted rather than
guessed — a claim about a reader we cannot identify has no place in the one
block whose instruction is to assert only what it read.

---

## 5. Speaking unprompted

Perception is what makes an agent able to start a sentence. It does not make
every observation worth one.

**Conditions register; the resident judges.** A condition declares what it
watches, what crosses, and — required — the verb the agent can offer:

```
storage-filling   free space trend crosses "full within 2 days"
                  → offer: scan for duplicates, back up to the Mac
```

Three rules, each earned:

1. **Fire on the crossing, not the level.** A condition true every day is said
   once, not daily. Otherwise the user learns to ignore it, which costs more
   than never having spoken.
2. **A "not now" holds.** The resident already declines per topic
   (`_declinedRecently`); a dismissed condition stays quiet until the situation
   materially changes, not until the next tick.

   The hold starts when the resident **speaks**, never when it is asked. A
   companion may answer SILENT, and a week of quiet bought by a sentence nobody
   heard is how a machine two days from full says nothing at all. Retrying
   costs nothing: the glance a notice rides was going to run a turn anyway.
3. **Every noticing ends in a verb the agent owns.** "Your storage is filling"
   with no next step is worse than silence. If there is nothing to offer, the
   condition does not qualify.

Silence remains the default. The resident's existing gates — foreground, no
pushed route, no song playing, time floors — still apply; perception gives it
better things to consider, not permission to interrupt more.

### Trends

Some conditions are about slope, not level: "full in a day" needs yesterday and
the day before. A daily sample of a handful of numbers (free space, roll size,
unbacked count), kept a couple of weeks, is enough. Written at rotation
(§7). Without it the best an agent can say is "your storage is nearly full",
which is the kind of line that makes an assistant feel dumb.

---

## 6. Two machines, one view

Each host writes its own log and computes its own state. Neither file is synced;
merging happens **at read time**, so there is no shared file to reconcile.

Each host publishes what it measured **about itself** on a retained
`perception` topic — the Mac under `mac`, a phone under `phone` — and each
renders the other's with its age. Published rather than requested, by the rule
app actions already settled: reads are published, actions are queued. The
phone is the reason it must be this way round — iOS suspends the app within
seconds of backgrounding, so a Mac that *asked* would be answered only when the
phone happened to be awake, which is exactly when it is least needed.

A host never republishes the other's lines, and never its own doorbell count: a
count means nothing to a reader who is not the one who last looked.

A host that cannot be reached simply contributes nothing. Its absence is itself
a state line, and its departure is a log entry.

**A newly arrived device has heard nothing.** A publisher that skips an
unchanged reading is measuring "unchanged" against a reader that may have just
connected, so the set of present devices changing is itself a change: the next
tick republishes. The arriving side does not wait for it either — the value is
retained, so it is fetched on connect, exactly as every other retained topic is.

**What crosses is cleaned by the reader.** Each host caps what its own writers
put in a record, because a bullet in the prompt is a line and a line is a
claim. That door governs one machine's writers. Lines arriving from the other
host were written by a door this one does not control and travelled a wire any
peer can hold, so the reader bounds them again — count, length, and control
characters, the host name included. A song title with a newline does not have
to be hostile to end a bullet early and start reading as an instruction.

---

## 7. Rotation and lifecycle

The log rotates daily. Rotation is one moment that produces two things:

1. **A state sample** — the day's numbers appended to the trend series (§5).
2. **The day's activities handed to the dream pass.** This is the same shape
   memory already uses: activity is to long-term memory what episodic is to
   semantic. The dream judges the day, promotes what is durable about the person
   to `ling-mem`, and the raw rows are swept afterwards. Activities do not get a
   second lifecycle of their own.

   One digest per day per host, never one row per activity — a memory store
   filling with "you deleted a song" is the failure this design exists to
   avoid. The phone's dream lives on the Mac, so a digest it could not hand
   over is held and offered again on the next connection: a day ages out only
   once.

**Sweeping is not a side effect of reading.** A day is dropped by the rotation
pass and only once its digest has been taken; until then it is offered again.
Rotation cannot be the thing that reads the log first — reading the log is what
starts it — so a log that swept on read handed every day to whoever happened to
open it, which was never the pass that had somewhere to put it. A failed handoff
and a day that never existed must not look the same an hour later.

Nothing is uploaded (§8).

Local retention is a few days, not one — perception wants "recent", and at 11pm
a one-day window holds almost nothing.

---

## 8. Nothing leaves the device

**The log is local, always.** It carries song titles, filenames, photo counts,
statement imports — and it exists so an agent can perceive the machine it lives
in, which is a question that never needs answering anywhere else.

Product analytics is a separate system and already does its own job: `/api/track`
records `screen_view` and `tap` with names only, capped at 64 characters and
validated server-side so content cannot travel. Perception adds nothing to it and
takes nothing from it. The two never meet.

So rotation (§7) has no upload step. A day is sampled for trends, handed to the
dream pass, and swept.

---

## 9. Settled, and open

Settled at build time:

- **One condition ships enabled** on each host — storage filling. The mechanism
  is general; each enabled condition is a licence to interrupt, so the next is
  added only after living with this one.
- **`play` stays out.** What is playing is state; what was played is a listening
  history, which is a different feature and belongs to DJ if it is ever wanted.
- **`backup-log.jsonl` is neither migrated nor read** — see §3.

Open:

- The Mac sweeps its conditions on Yinyue's ambient glance and the phone on
  resume, because both are moments she could speak anyway. Neither host notices
  a crossing while nobody is there to be told — right for a companion, wrong the
  day a condition needs to act rather than speak.

---

Related: [memory-spec](memory-spec.md) (where durable facts go after the dream),
[yinyue-companion-spec](yinyue-companion-spec.md) (the resident, its gates, and
what it does with a notice), [app-action-spec](app-action-spec.md) (the verbs a
noticing can offer), [network-spec](network-spec.md) (how a host reads the
other's perception).
