---
type: spec
reader: Coding agent and users
guide: |
  Product specification — describe what the system should do and why.
  Keep it brief. Aim to guide design and implementation, not document code.
  Avoid implementation details like function signatures, variable types, or code snippets.
---

# Memory

Memory V2 is Linggen's memory. Linggen grows up with the person: years later it
still knows what they did and how far they came, and it knows them well enough
to be a companion, not only a tool (product-spec, principle 0). Models get
stronger, and each tool's own memory stays inside that tool and short-lived;
the person's long record, kept on their own devices, is what Linggen holds that
nobody else will. So memory is small, plain and the person's: Markdown files
they can read, edit and delete.

**Is:** facts about the person, rules they state, decisions with their why,
gotchas they would re-hit, dated life moments, and a monthly chronicle of what
they did. **Isn't:** an activity log (git records that), where work stands (the
docs hold that), a codebase snapshot (files are truth), a transcript (the
session store has it). Secrets never go in. Memory never writes to project
files.

**Other agents' memory is read, never written.** Claude Code and Codex keep
their own memory their own way. Linggen reads it, never edits, moves or copies
it, and tells each agent what the others remember. Linggen writes only its own
notes: what Ling and Yinyue keep about the person and the work.

Code: `crates/linggen-engine/src/own_memory/` (the notes) and
`crates/linggen-engine/src/outside_memory/` (other agents' memory).

## The store

Plain Markdown, UTF-8, under `~/.linggen/memory/`:

```
person/<topic>.md                    about the person
areas/<area>/<topic>.md              area: body, mind, people, music, money, play, home
projects/<encoded path>/<topic>.md   work, per project folder
moments/<yyyy>/<yyyy-mm>.md          Yinyue's dated life moments
chronicle/<yyyy>/<yyyy-mm>.md        the chronicle, a month; <yyyy>.md is the year
.state/                              prior versions, use counts, the move record (local only)
```

A project folder is named as Claude Code names it: every character that is not
an ASCII letter or digit becomes `-`. A note filed under a project reaches
sessions in that folder and below. Nothing else under `memory/` is read as a
note (the v1 store keeps its database and backups there).

**One topic per file, one note per `##` entry.** A topic file starts with its
name and scope; each entry is a one-line summary, a meta comment, then the body:

```
---
topic: Build and test
scope: project:/Users/alex/work/app
---

## Run tests with just test
<!-- id: 3f9a2c71b0 · kind: rule · voice: user · created: 2026-11-24 · updated: 2026-11-24 · source: claude-code -->
Plain `cargo test` leaves the fixtures stale; `just test` rebuilds them first.
```

| Field | Meaning |
|---|---|
| `id` | 10 characters; a note keeps it for life, so files merge by entry |
| `kind` | `person`, `rule`, `decision` (with its why; "tried X, dropped because Y" belongs here), `gotcha`, `moment`, `chronicle` |
| `voice` | `user` (the person's own words) or `agent` |
| `created`, `updated` | days, `YYYY-MM-DD` |
| `source` | who wrote it: `claude-code`, `codex`, `linggen`, `yinyue`, `phone`, … |
| `void: <why>` | known wrong; shown only in Settings |
| `no-raise` | Yinyue never brings it up herself |
| `yearly: MM-DD` | a day that repeats every year (`YYYY-MM-DD` when the first year is known) |

Keys a build does not know are kept, so another writer's survive.

**Limits** (Claude Code's own): a summary is at most 80 characters, a body at
most 4,000, a topic file under 200 lines and 16 KB. Past that the oldest notes
move to `<topic>-part2.md`, then `-part3`, and on. A moment goes only in
`moments`, a chronicle line only in the chronicle, every other kind anywhere
else.

Every save is atomic, and the old file is kept under `.state/history` (the last
20 versions of each file).

## What a session sees at start

The engine makes an index from the entries' meta comments; nobody keeps one by
hand, so no note falls out by accident. A session of Ling or Yinyue has the
index in its prompt, a few lines on how to use notes, and the `memory_notes`
tool. It has no per-turn recall and no capture reminder: what is worth keeping
is written at the moment it appears, and what is not in the index is one search
away.

- **Line:** summary, kind, topic and id ("don't raise" when set). Void notes and
  the chronicle are not listed.
- **Order:** the person's notes (all of them); then the session's place (the
  folder's notes, nearest first, up to but never the home folder; for Yinyue,
  her areas); then notes read in the last 90 days, most read first; then the
  rest, newest first.
- **Budget:** 200 lines and 25 KB, Claude Code's own limit for its index. What
  does not fit is forgotten, not gone: the index says how many, `op=search`
  finds them, and reading a note brings it back into the index.
- **An app's session** gets only the notes filed at its own folder or above, and
  the person's when it has core, and no tool.
- **Yinyue's day** is added to her prompt alone (see the chronicle section).
- **A mission** has no notes, except the chronicle's own run.

## The one writer

`memory_notes op=write` is the only thing that saves. Ling, Yinyue, the phone, an
outside host with no memory of its own, Settings and the move from v1 all go
through it, so its checks hold for everyone:

- **Only memory.** A rule, a decision with its why, a gotcha, a fact about the
  person, or (Yinyue's) a moment. Status, progress and "what was done" are
  refused with one fixed sentence a caller can pass on. A chronicle line is
  exempt: what the person did is what it is for.
- **Actions:** add, update by id, void (with a reason), no-raise, yearly. The same
  summary twice in a topic is refused, with the id of the note that has it.
- **The person's own words change only with the person.** A `voice: user` note
  is shown to them first; after they agree, the caller passes `user_directed`.
- **Merge by entry.** The caller says which version of the file it read. If only
  other entries changed, the write merges; if the same entry changed, it is
  refused with the current text.
- **The fence applies:** a caller cannot write where it cannot see (below).
- **Un-void and delete are the person's, from Settings.** The tool has no word
  for them, so an agent cannot bring back a note the person marked wrong or
  remove one.
- **A topic name belongs to one scope**, and two writes never run at once.

## Reads and use counts

`memory_notes` has four ops: `index` (the notes for a folder), `read` (one note
by id, or a topic file by path; a void note is read and says so), `search` (words
in any language) and `write`. Search ranks a note by how many of the words it
holds, newest first among equals; a run of Chinese is matched two characters at
a time. It never finds void notes, and it also returns what other agents'
memory says on the words, under `other_agents`.

A note that is read or found counts a use (`.state/uses.jsonl`, local, never
synced). Uses are what keep a note in the index for 90 days. The index itself and
Settings count none, or everything shown would count as used.

## The privacy fence

What the person says about their body, their mind and the people close to them,
Yinyue's moments, and the chronicle (which holds all of those in a line a
month) never go to a coding host. One decision, by audience:

| Audience | Who | Sees | Writes |
|---|---|---|---|
| Companion | Yinyue, outside a coding session | everything | everything |
| Work | Ling, apps, Claude Code, Codex, any host on the door; Yinyue in a coding session | person, projects, areas music, money, play, home | the same |
| Chronicler | the chronicle's own run | everything | the chronicle only |

The fence sits at every way out of the store: the index, a read by id or by
path, a search hit, and the lookup a write does by id. A fenced note answers "no
such note", the same as a missing one. A test fails when any other code loads a
topic for a caller. Settings → Notes reads as the owner, who sees all of it.

## The tools

`memory_notes` and `memory_agents`, defined once for Linggen's sessions and the
MCP door (`mcp-spec.md`; the catalogue lists every tool, and no doc counts them).

- **`memory_notes`** — Linggen's own notes, the ops above. One definition of its
  text and fields serves a session and the door. On the door it answers only the
  owner at this Mac, when `outside_agents` is v2; the host is always a Work
  audience and is recorded as the writer. What goes here from an outside host:
  facts about the person that hold in every project (language, how to address
  them, long-standing preferences), scope `person`. Project things (code
  conventions, project decisions, gotchas) go to the host's own memory.
- **`memory_agents`** — what other agents remember, read only: `mode` (v1 or v2),
  `index` (the session-start block for a folder: Linggen's own notes, then their
  index lines, never the calling host's own), `read` (one note by path, only inside
  a declared place) and `search`. The owner, at
  this Mac only.

## Outside agents and forwarding

Claude Code and Codex write memory their own way, and keep changing it;
Linggen does not touch it. It reads each agent's files where they are and
forwards what they say to the other agents. It never writes, tidies or
summarises another agent's memory, imports nothing, and asks the person for
no step: what cannot be read from disk (a vendor's cloud memory) is not read.

### Readers

An agent's memory is described by one **reader**, a declaration file (TOML,
`format = 1`): where its notes are, how a file becomes index lines, which lines
are about the person and which about a project, and which files are only
opened on demand. The engine owns the closed set of primitives (path
variables, line extractors, scope rules, version guards); agents and their
formats are open data. A vendor renaming a heading is a reader update; a
format that needs a new primitive (a database, another folder encoding) is an
engine release.

- **Two sets, merged by the agent's id.** The Claude Code and Codex readers are
  embedded in the engine, the floor an offline or fresh install forwards from.
  A skill in `~/.linggen/skills/` may carry one (`memory-reader: reader.toml`
  in its `SKILL.md`, see `skill-spec.md`); it replaces the embedded reader of
  the same `agent`, or adds a new agent. Nothing else counts: a project's
  `.linggen/skills/` and the other agents' skill folders can be written by
  another agent, and a reader chooses what Linggen reads. A skill folder that is
  a link is skipped.
- **What a declaration may name.** Paths under the home folder (`~/…`), `*` in
  a name (never matching a dot name), `{project}` for one folder of the session's
  chain; never `..`, never `~/.linggen` (Linggen's own store has its own fence),
  only `.md`, `.markdown` and `.txt` files, depth and size caps. A reader that
  breaks a rule, does not parse, or says a newer `format` is refused whole and
  the embedded one stays; the refusal is kept for Settings.
- **A place** is one kind of file: `id`, `path`, `scope` (`person` or
  `project`), `lines` (`each` = `item`, `text`, `heading` or `file`, with
  `depth`, `first`, `read_first_lines`, `read_first_bytes`, `newest`, `text`,
  `note_link`), `parts` (sections of one file with their own scope, a project
  section split by headings that name a folder), `expect` (the vendor's version
  guard: its first line, or a heading), `group` (places with the same group:
  only the one whose files changed last is forwarded), `forward = false` (opened
  and searched, never listed). Its address is `agent/place`
  (`claude-code/project-index`).
- **Where a file is read.** One file touches the disk (`outside_memory::disk`),
  and a note is read only when, once links and `..` are resolved, it is a file a
  declared place names: the same refusal for "not there" and "not allowed", and
  every outgoing string passes the redaction (key-shaped strings are replaced).
- **Drift is said, not hidden.** A place whose `expect` fails forwards nothing
  and reports it ("Codex: format not known: first line is 'v2', expected
  'v1'"); so does a place whose file has content and yields no line ("nothing
  matched"). Reading and searching stay allowed: confinement does not depend on
  understanding the format. A missing or unreadable place is skipped and shown
  as such, never fatal.
- **No scripts** in this version: none of the formats needs one.

### What the built-in readers read

- **Claude Code:** each folder of the session's chain has a project index
  (`~/.claude/projects/<folder>/memory/MEMORY.md`, read as Claude Code loads it:
  the first 200 lines or 25 KB), forwarded for that project; the notes it points
  to are opened and searched; `~/.claude/CLAUDE.md` (its headings) is the
  person's.
- **Codex:** `memory_summary.md` in `~/.codex/memories/` or, newer,
  `~/.codex/memories_v2/` (the one that changed last; it must begin with the line
  `v1`): its person sections (`User Profile`, `User preferences`, `General Tips`)
  are the person's, and the project blocks under `What's in Memory` (a `###` or
  `####` heading naming a folder of the chain, by path or by name; a dated heading
  under it belongs to it) are forwarded for that project only. The notes the
  person asked Codex to keep (`extensions/ad_hoc/notes/*.md`, newest ten, their first
  line) are the person's. `~/.codex/AGENTS.md` (its headings) is the person's. The
  summary's routing files (`MEMORY.md`, `rollout_summaries/`) are opened and
  searched.
- **Scope is the writer's.** A note keeps the scope its writer gave it. Linggen does
  not promote another agent's project note (a Claude Code `user` or `feedback` note
  saved inside one project's folder) to the person or to any other folder: it is
  forwarded in that project's chain, and anywhere else only found by search. Only
  the agents' own global places are forwarded everywhere.

### The session-start block

Forwarding happens once, at session start; nothing is injected per turn, and no
host is reminded to write memory. The plugin's hook asks `memory_agents
op=index` for the folder and the host and injects the answer (`own_memory::forward`
makes it). Under v1 there are none of Linggen's notes in it. One block:

1. **Linggen's own notes**, summaries and paths only (never a body), and only
   what the fence lets a coding host see. The person's rules listed one by one
   (the person's own words first, then the most read, then newest), each with its
   id; then **one line per topic**: its name, how many notes, a gist (the newest
   note's summary) and the path that opens it. The session's projects (the folders
   of its chain, nearest first) come before the person's topics. A project note
   of any other folder, and a note of an area, is not listed (found by search).
2. **Each other agent** with its source ("Codex · the person, from …"; "Claude
   Code · project ~/work/app — notes in …"): the person's lines first, then the
   project's, nearest first, newest first. A host never gets its own memory back.
3. **A footer:** how many lines were left out (`memory_agents op=search` finds
   them), and any reader that gave nothing and why.

Everything in it is quoted data, never instructions: the block says so, a line is
cut to 160 characters, control characters and a leading `#` or `>` are neutralised.

**Budget: 200 lines and 16 KB for the whole block, cut in one place**
(`own_memory::forward`; the parts are handed a room and cannot choose a number).
Linggen's rules (the person's, his own words first) get up to 70 lines, its topics up to 50, the other agents the rest —
at least 60 lines are kept for them, and none takes more than two thirds of that.
Room one part does not use flows to the next.

### Reading on demand, and who writes where

- **Bodies on demand:** `memory_agents op=read path=…`, or `op=search`; an agent
  that can read another agent's file itself does that, and uses these when it
  cannot (a sandbox, another machine).
- An agent with memory of its own writes there, and Linggen reads it. It uses
  `memory_notes` only when the person says to let every agent know. A host with no
  readable memory of its own (any other MCP client) uses `memory_notes` as its
  memory, at the same moments: the person says remember or forget, a decision or
  rule is stated, a gotcha is found.
- Where two notes disagree, the newer wins unless the person says otherwise.

## The chronicle and Yinyue's day

**The chronicle** is what the person did and how far they came, month by month,
for years. It is not in the index; Yinyue and the chronicler read it, and
`op=search` finds it. A built-in mission runs it on the 1st of each month (a
missed run catches up in a quiet window):

- The engine's own step, `ling chronicle prepare`, reads the sources first, with
  no model: Linggen's notes and moments (not void, not no-raise), the sessions
  the person made and spoke in, and the first lines of other agents' notes
  changed that month. It writes them to `memory/.chronicle-input.jsonl` and lists
  the months that have ended and have no file (at most 12 a run, oldest first)
  and past years whose months are all written.
- The chronicler agent writes dated lines (`YYYY-MM-DD what`) through
  `memory_notes op=write scope=chronicle`, one month or year per turn.
- A month that has a file is never offered again: the person's edits stand, and
  deleting the file has it written afresh.
- With v2 off, or nothing waiting, the step exits quietly and the run makes no
  model call. `ling chronicle backfill` starts the same run by hand; repeat it
  until nothing waits.

**Yinyue's day** is built for her prompt alone, as facts: recurring days (a
note's `yearly` day) within the next week, and moments and dated chronicle lines
from this date in past years (at most six). No-raise and void notes never appear,
and her guidance says she never raises a no-raise note herself. Whether, when and
how gently to speak is hers.

## The phone

The phone is unchanged: it still sends `add`, `delete`, `search` and `list` to
`/api/memory/<verb>`. The door answers those four for the owner's own phone from
the notes. Every other caller (a phone that is another person, an approved
computer, a skill page) stays on the v1 store, because the notes have no
per-person partition.

- **A phone note** is one write with source `phone`, filed by app: `yinyue` is
  `area:mind` (also the default for a note naming no app, so coding hosts never
  see it), `dj` is `area:music`, `cfo` `area:money`, `health` `area:body`,
  `shifu` `area:home`, `photos` `area:play`.
- **`list`** is the pull: the person's notes as `core`, the areas' notes as
  `semantic` under the app's folder. Moments, the chronicle and work notes are
  not the phone's.
- **A search that carries `min_score`** is the old per-turn recall. v2 has none,
  so it finds nothing.
- **A note that can never land** (status, empty) is answered ok with no id, so the
  phone's queue does not stop on it. Only a failure worth retrying (the disk, a
  note changed under the write) is an error.
- Run the move from v1 first: rows that carry an old-store id are not in the
  notes.

## Settings → Notes

The owner sees and tends what Linggen remembers, by source: Linggen's own notes,
and each agent the registry reads (the tabs come from the registry; the page
names no agent).

- **Linggen's notes** can be edited, marked wrong and right again, kept from being
  raised, and deleted (the file's last version stays in history; a file left with
  no notes goes). Each is one call to the one writer as the person
  (`user_directed`, so a note in their own voice changes too).
- **Other agents' notes** are listed and read (redacted, with their path). No
  route changes them: the routes that change a note name a note id and no
  source. All are admin.
- **A review list** shows notes marked wrong, and notes neither read nor changed
  for 90 days. The chronicle's lines appear only when marked wrong: they are
  never read into an index, so "not read" says nothing of them. Looking counts no
  use.
- Settings → Memory remains the page for the v1 store.

## The switch

Two settings in the engine's config, each `"v1"` or `"v2"`, absent meaning v1.
V2 is opt-in in 2.7; write `v2` to try it:

```toml
[memory]
linggen = "v2"          # Ling, Yinyue, apps, the owner's phone, the chronicle
outside_agents = "v2"   # Claude Code, Codex and other hosts on the door
```

`linggen` says which memory Linggen's own sessions have, and whether the phone's
four verbs and the chronicle use the notes. `outside_agents` says how hosts
outside Linggen are served: v2 forwards other agents' memory at start, serves
`memory_notes` and **stops offering the five v1 memory tools on the door**
(`mcp-spec.md`: the door is the view and the mode; an agent's own memory is the
place for what it learns, and a write through `memory_add` would land in a store
nothing reads); v1 is the plugin's old recall and capture. The two are
independent. Writing `v1`, or taking the section out, is the way back. The
settings are hand-edited; a saved settings page keeps what the file holds.

## Moving over from v1

Rule-based, no model, through the one writer (the same checks as an add, the
row's id and dates kept, one save per file). The v1 store is only read.

- **Kept:** a preference (becomes a rule), a decision, a learned gotcha, and a
  fact about the person. A row with no scope is about the person; a folder scope
  is that project. Voice is `user` where the row is from the user.
- **Not kept**, each counted by reason: what was done (built, fixed, tried),
  episodic staging, a fact about a project, a row another person's phone wrote or
  v1 already replaced, a key-shaped secret, and what the writer refuses (reads as
  status, too long, same summary in the topic).
- **Topic names** are the first word of the summary's subject (two when it is
  short), so the same row always lands in the same file. The rules let more
  through than a judged pass would; thinning what remains is later work.
- **By hand:** `ling mem migrate-v2 [--dry-run] [--from export.ndjson]`. Safe to
  run again: a row whose id is already a note is left as it is. `--dry-run` prints
  the counts of a real run and writes nothing.
- **Once, at the engine's start:** when either switch is v2, no note exists and
  the v1 store has rows, the move runs, and `.state/moved-from-v1.json` records
  it. A Mac that already has notes is recorded and left alone. Both switches `v1`
  skip it. A failed read records nothing, and the next start tries again. Delete
  the record to have it looked at again.

## Not built yet

Entry-by-entry sync with the phone (each note its own unit, newest `updated`
wins per entry, void travels as an edit; the phone's side is in
`phone-memory-spec.md`). Other agents' session logs in the chronicle. Suggesting
that a note used again and again move into a project's `AGENTS.md`.

## V1 (the way back)

`linggen = "v1"` and `outside_agents = "v1"` (or no `[memory]` section) is
what ran before Memory V2. The
v1 store (ling-mem rows) is still opened and served by the engine under v2, for
what v2 does not cover: a phone that is another person, approved computers, skill
pages, and the move above. The built-in dream, scan and solve missions
(the missions of the `memory` agent) tidy that store, so under `linggen = "v2"`
each run is skipped with a plain reason — no model call, no session — by the
`v1-only` step they open with (`src/cli/v1_only.rs`, `doc/mission-spec.md`
"prepare"). Under v1 they run as before; the chronicle is v2's own and runs
only under v2. With `linggen = "v2"` but `outside_agents = "v1"`, outside hosts
still read the v1 store, which these missions no longer tidy.

- **Tiers.** Core (a handful of high-confidence universals, in every session),
  long-term (durable state and lessons, never events), short-term (per-turn
  staging that fades after about a week once judged).
- **Per session.** Capture protocol and reminder, per-turn recall ("From memory:
  …"), the always-on identity block, scope (a row's `scope` is the folder it is
  about) and an index of standing rules. Exact duplicates collapse at write time;
  the agent merges its own notes freely, but a row in the user's voice changes
  only with the user. Tools: `memory_search`, `memory_add`, `memory_update`,
  `memory_delete`, `memory_upkeep`.
- **Offline judgment.** The nightly **dream** remembers each day's staging, lets
  the rest fade, and audits long-term rows (merges archive, never delete; what
  needs the user becomes a review item). **Scan** backfills a day from host
  session logs; **solve** drains review items with the user. Both are missions on
  the `memory` agent, started from Settings → Memory.
- **Rules.** The user's voice changes only with the user; record what was said,
  never invent; the file beats the memory; curate, don't accumulate; never store
  secrets; status rows are perishable.

### Hosted by the engine

`ling` opens the v1 store at start-up; there is no separate `ling-mem` process.

- **One writer:** the engine holds `memory/.store.lock` for its life; a second
  opener is refused. `ling mem <verb>` goes to the engine when it is up, else to
  the store directly under the lock.
- **Doors, one port:** `/mem/*`, `/mcp` (the memory tools among the door's;
  `/mcp/memory` is retired and answers 410) and the phone's `/api/memory/<verb>`.
  Nothing listens on the old daemon's 9528, and there is no address to set.
- **The old binary** `~/.local/bin/ling-mem` is a stand-in whose data verbs run as
  `ling mem`. A running old daemon is stopped first, and each engine version
  copies the store to `memory/backups/takeover-<stamp>/` before it first opens it.
- **Warm-up.** A call that embeds waits up to 20 seconds for the embedding model,
  then is refused with how long memory has been warming; it is neither applied nor
  queued. Automatic recall skips quietly at once.
- **Who reaches memory** (decided once, in `memory_host::caller`): this machine
  and an approved computer (`machine`) see all of the owner's memory; a `phone`
  sees its person's rows only and runs only the per-person verbs (`search`,
  `list`, `count`, `get`, `add`, `add_batch`, `update`, `delete`, `forget`,
  `session_start`); no token or an unknown one, nothing. Whose rows comes from the
  caller, never the request body, and a write stamped with a person may name only
  that person's rows. Allow and Deny happen only at this machine.

### Across machines

One person, one memory: the stores form a star with one **primary** and any
number of **replicas**. A machine picks its primary in Settings → Memory → Other
machines: local (the default) or another machine's Linggen (`host:port`, its
engine port). A replica asks to join; the primary's machine shows Allow / Deny.
Reads come from the local copy; writes go to the primary, queued while it is
away; the primary wins a conflict and files a review item. Dream and solve run on
the primary only. A machine with only the plugin points at another machine's
`/mcp` with that machine's token (`/linggen:config`).

### Scan and solve

Both are missions with no schedule. Scan: `POST
/api/missions/memory-scan/trigger {"day":"YYYY-MM-DD","attended":true}`; the
engine first runs `ling mem scan <day>` (`cli.md` § mem: session logs of Claude
Code, Codex, OpenClaw and Linggen, secret-filtered, no model), then the agent
stages candidates as short-term rows and stamps the day. Solve: `POST
/api/missions/memory-solve/trigger {"attended":true}`; it settles items from
evidence and asks the user only when evidence cannot. A trigger answers 200 with
the run's session, 409 while the same mission is running, 400 for a malformed
day. Missions never prompt: a call outside their grants is refused.

### Where the v1 detail lives

Capture protocol: the engine system prompt and the plugin's MCP instructions ·
offline judgment, scan and solve procedures: `agents/memory.md` and `missions/` ·
store schema and CLI: `linggen-memory` docs · layout: `storage-spec.md` · tool
dispatch: `tool-spec.md` · the phone's memory and the account stamp on its rows:
`phone-memory-spec.md`.
