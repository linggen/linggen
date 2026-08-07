# Perception Spec

**Status: designed, not built.**

What a resident agent knows about the world it lives in, without being told.

Yinyue lives in a phone. Ling lives in a Mac. Each should be able to answer
"what is true here", "what just changed", and "who did it" — and to speak up
when something is worth a word. Today neither can: they know only the
conversation. That gap is not cosmetic. It is why Yinyue offered to sync songs
that had already arrived — nothing in her world said they had.

Perception is that world, in three parts that cost three different amounts.

---

## 1. The three parts

| | Answers | Costs | Lives |
|---|---|---|---|
| **State** | What is true now | Every turn | Recomputed, never stored |
| **The doorbell** | Has anything happened | Every turn, two lines | Derived from the log |
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

**Source.** The device lines come from the readout Shifu already measures
(`ReadoutPublisher` on the phone, `data/readout.json` on the Mac). App lines come
from the app's own controller. No new measurement subsystem: perception reads
what the app already knows, so the agent and the screen can never disagree.

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
  second one.
- `app` — `dj`, `photos`, `cfo`, `shifu`, `system`.
- `verb` — a short closed-set slug: `delete`, `add`, `edit`, `sync`, `backup`,
  `clean`, `import`, `play`, `pair`, `connect`, `disconnect`.
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

### Conditions vs. transitions

A condition belongs in state; its **transition** belongs in the log. "No Mac
right now" is state, and it stops the agent offering a sync. "Lost the Mac at
11:03, back at 11:14" is a log entry, and it is what explains why something
failed while the user was away. Both are needed and they are not the same fact.

### Where it lives

- Phone: the app's documents directory, beside the app data it describes.
- Mac: `~/.linggen/activity/`, one file per day.

The Mac's `backup-log.jsonl` — already JSONL, already rendered as "Activities"
in Shifu's Media pane — is the same idea in one lane. It folds into this rather
than sitting alongside it.

---

## 4. The doorbell

Two lines in the state block, derived from the log at build time:

```
since      6 things happened since you last looked
latest     you deleted 三天三夜 · 3 minutes ago
```

That is the whole always-on cost of history. It gives the agent the hook to
remark and, most of the time, the answer as well. When it needs more it calls
`recent_activity` — a tool, so it costs nothing until wanted, never enters the
window uninvited, and never renders in the thread.

"Since you last looked" is per-agent, stamped when that agent last read.

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

An agent reads the other machine's perception the way it reaches anything over
there — the peer request path, by the capability-symmetry rule: the tool lives
where the action lives, and the peer gets a request tool. Yinyue asks the Mac,
Ling asks the phone.

A host that cannot be reached simply contributes nothing. Its absence is itself
a state line, and its departure is a log entry.

---

## 7. Rotation and lifecycle

The log rotates daily. Rotation is one moment that produces two things:

1. **A state sample** — the day's numbers appended to the trend series (§5).
2. **The day's activities handed to the dream pass.** This is the same shape
   memory already uses: activity is to long-term memory what episodic is to
   semantic. The dream judges the day, promotes what is durable about the person
   to `ling-mem`, and the raw rows are swept afterwards. Activities do not get a
   second lifecycle of their own.

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

## 9. Open

- **Which conditions ship enabled.** The mechanism is general; the starting list
  is a product choice, because each enabled condition is a licence to interrupt.
  Recommendation: exactly one — storage filling — and add the next only after
  living with it.
- Whether `play` deserves to be an activity after all, for "what did I listen to
  this week".
- Whether the Mac's existing `backup-log.jsonl` is migrated or simply read in
  place during the transition.

---

Related: [memory-spec](memory-spec.md) (where durable facts go after the dream),
[yinyue-companion-spec](yinyue-companion-spec.md) (the resident, its gates, and
what it does with a notice), [app-action-spec](app-action-spec.md) (the verbs a
noticing can offer), [network-spec](network-spec.md) (how a host reads the
other's perception).
