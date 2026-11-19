---
type: spec
reader: Coding agent and users
guide: |
  Product specification — describe what the system should do and why.
  Keep it brief. Aim to guide design and implementation, not document code.
  Avoid implementation details like function signatures, variable types, or code snippets.
---

# Memory

Persistent knowledge that travels with the user across sessions and
hosts — who they are, how they want to work, the decisions they've
made. Memory helps every kind of user, not just coders.

**Is:** identity, preferences, decisions with their reasoning, gotchas
the user re-hits. **Isn't:** an activity log (git records that), a
codebase snapshot (files are truth), a transcript (the session store
records that). A candidate that earns no place is dropped. Memory
never writes to project files.

## Parts

- **ling-mem** — standalone binary + local daemon; owns the store and
  all mechanical operations (dedup, search, export). No LLM inside;
  every frontend goes through it, so deleting a frontend never loses
  data.
- **Linggen engine** — memory tools for every agent, the always-on
  identity block in each session, per-turn recall, and the capture
  protocol in the system prompt.
- **Memory agent + missions** — the offline judgment brain; runs the
  nightly **dream** mission (remember → forget → condense) and the two
  manual operations, **scan** and **solve** (missions `memory-scan`,
  `memory-solve`). Settings → Memory in Linggen's UI shows rows, days and
  the review queue, and starts them.
- **linggen plugin** — the same store in Claude Code and Codex:
  recall each turn, capture protocol, runbooks, and `/linggen:scan` for
  backfilling days. Third-party hosts reach the store through the engine's
  `/mcp/memory` endpoint (`mcp-spec.md`). Any MCP-capable agent can add
  `http://127.0.0.1:9527/mcp/memory` as an MCP server (tools only).

## Tiers

- **Core** — a handful of high-confidence universals about the person
  (name, role, location, family), present in every session. Never
  scoped, never a work rule.
- **Long-term** — everything else durable, retrieved on demand. Holds
  *state and lessons, never events*: would the row still matter in
  three months?
- **Short-term** — per-turn working capture. Events and uncertain
  signal land here; once the dream pass has judged a day, its rows
  fade after about a week unless promoted.

## Features

- **Capture** — the live agent saves signal in the same turn it
  appears, on every host. Explicit statements go straight to
  long-term; incidental signal stages in short-term.
- **Recall** — relevant memories surface at the start of each turn;
  facts used in a reply are cited ("From memory: …"). The identity
  set is always present.
- **Scope** — a row's `scope` is the directory it is *about*
  (ling-mem `doc/scope-index-spec.md`); none = about the person. The
  host stamps the session's `cwd` (the default scope) and `root`; at session start it shows
  "Memory scopes here: …" and the model picks another one as `scope`
  when the row is about it (the daemon validates). Recall sees rows
  under the session root, at its parents, and about the person. A
  skill's own session stands in `~/.linggen/skills/<name>` and sees
  only its rows (`memory-context:` is now just the skill's opt-in to
  memory). Home, `~/.linggen` and temp see rows about the person, plus at most two
  strong matches filed under a directory (cosine ≥ the daemon's
  `no_root_project_min_score`, 0.70; never preferences).
  Rows carried forward by the dream or a backfill keep their scope.
- **Index** — standing rules carry `indexed` and a one-line `summary`:
  every session in their directory or below loads the summaries at
  start (nearest first, 3000 chars; no summary → the content's opening),
  and reads a row in full when its summary bears on the task. The dream
  fixes scope, index and summary itself when sure (`memory_update`, ≤ 10
  per run, a `FIX` line each; never content); unsure → left alone. No
  one reviews scope by hand. The engine reloads the index when the
  session's cwd changes.
- **Dedup** — exact duplicates collapse automatically at write time.
  Anything fuzzier is judgment, not mechanics.
- **Reconcile** — authority follows voice: the agent freely merges
  and rewrites *its own* notes into current truth; anything the
  *user* said changes only with the user (ask first). The store
  itself enforces the floor — a silent rewrite of the user's voice
  is refused, on every host. Whoever sees garbage fixes it in that
  moment — there is no cleanup queue.
- **Dream** (nightly) — reviews each day's short-term staging,
  promotes what's durable, and lets the rest fade. Never deletes
  unjudged rows. Day-by-day, with a visible per-day state on the
  calendar. Ends with the condense stage once the worklist is clear.
- **Scan** (user-triggered) — backfills a past day from host session
  logs, for days live capture missed. Safe to re-run: sessions that
  already contributed are skipped. See **Scan and solve** below.
- **Audit** (dream's last stage) — cures stale long-term memory by
  confidence: what the agent can solve it solves, the rest is queued
  for the user. Two confident lanes, both capped per night, store
  snapshot before every run: condense — high-confidence cited chains
  of its own notes collapse into one current-truth row each — and
  completion-bar marker merges — a provisional-state note whose
  strictly newer same-subject derived neighbor asserts the work done
  collapses the same way (the store already holds the answer).
  Everything else the dream **settles itself from evidence**
  (2026-11-09) — the rows in full, a newer row on the subject, and
  code or git where the run has those tools — the way solve does:
  derived rows are fixed in place (or left as they are), and the
  ruling is recorded as a review item closed at once (resolved or
  dismissed, with a one-line why). It queues an open **review item**
  only when a user-voice row would change, the user must pick between
  rows, or the evidence is out of reach on that run. The dream mission
  holds READ over the home directory with Read / Glob / Grep / Bash:
  it looks at code, docs and git history, and the engine refuses any
  write (missions never prompt). The marker scan excludes rows any review
  item names, open or closed, so a settled or queued candidate is not
  served again.

  Merges ARCHIVE, never delete (2026-08-17): a `replace_ids` loser in
  the semantic table gets `expired_at` + `superseded_by = <survivor>`
  — invisible to search, list, scans, and counts, kept on disk, so
  every merge and digest can be unpacked (`list --superseded-by
  <id>`; `--include-expired` widens any read). Episodic losers stay
  hard-deleted: staging is disposable. The stats surface reports the
  archive count — archived state is visible state.

  The third confident lane, **subject digests**, runs in the dream
  too: the `subject` detector (cosine star clusters, 3–12 members)
  serves only QUIET clusters (newest member >30 days — a live
  subject keeps its detail) and skips rows a `subject`-kind ruling
  covers. The judge digests clusters it is confident share one
  subject (one digest row, ≤5 per night — a digest is known by the
  rows whose `superseded_by` points at it), rules clearly separate
  clusters itself (a dismissed `subject` item), and queues only the
  ones it cannot tell as open `subject` review items — listing ALL member
  ids, so a ruled cluster can never re-form. In solve, the user's
  keep-separate answer becomes the permanent (dismissed) ruling.
  Digesting unattended is safe because merges archive: a wrong
  digest is an unpack, not a loss.
- **Solve** (attended, the rare leftover path) — a host agent drains
  what the dream could not settle, with the user present: gathers evidence at solve time (git history,
  files), fixes what the evidence proves, asks the user one item at a
  time for the rest, closes each item. Surfaces: `/linggen:solve` on
  plugin hosts; in Linggen the `memory-solve` mission, started from
  Settings → Memory, which shows the queue. Dream reports and the recall
  footer carry the open count.
- **Across machines** — one memory shared by every machine the user
  works on; see the section below.
- **Secrets** — credentials never enter memory; deleted on sight.

## Scan and solve

Both are built-in missions on the `memory` agent with no schedule: they run
only when started, the way a calendar day's dream is. Their procedures are
in `agents/memory.md` ("Procedure — scan one day", "Procedure — solve the
review queue"); the mission files carry only the pacing and the grants.

- **Scan a day** — `POST /api/missions/memory-scan/trigger`
  `{"day":"YYYY-MM-DD","attended":true}`. Before the first turn the
  engine itself runs `ling mem scan <day>` (the mission's `prepare` step;
  `cli.md` § mem: the day's session logs of Claude Code, Codex, OpenClaw
  and Linggen, flattened and secret-filtered into
  `~/.linggen/memory/.scan-output.jsonl`; no model, no network); if that
  fails the run ends failed, unstamped, with no model call. The CLI reads the
  Mac's local account even with the engine stopped and supplies the owner
  identity to the collector, which skips other people's Linggen sessions
  (`skipped_other_person` in the summary); signed out, only local or untagged
  Linggen sessions qualify. The transcript includes Yinyue's replies to
  a direct address or an explicitly requested moment as assistant text,
  not unsolicited lines, hidden prompts, tool observations or compaction
  summaries. The agent then
  checks the file's header is that day's, lists the candidates, skips
  every session that already has rows for the day (by `source_session`),
  judges one candidate per turn and stages what is worth keeping as
  short-term rows, each carrying its session id and the session's
  directory. It ends by stamping the day scanned (`harvest_day`), also
  when nothing new was staged; a missing or stale file ends `FAILED`,
  unstamped. It never promotes:
  the next dream judges the rows. Today can be staged but not stamped. A
  day with more sessions than one run's turns ends `PARTIAL`, unstamped;
  scanning again continues. Scan works on every machine.
- **Solve** — `POST /api/missions/memory-solve/trigger` `{"attended":true}`.
  Works the open review items from evidence (rows, files, docs, git
  history — read only); asks the user one plain question with a
  recommendation only when evidence cannot settle an item; a row in the
  user's words changes only after their answer. `attended` puts AskUser in
  the run's tools; without it, items needing the user stay open. On a
  replica it stops at once: solve runs on the primary.
- **What the trigger answers** — `200 {"ok":true,"message":"Mission
  triggered","session_id":"<id>"}`: the run's session, open at once, where
  the user watches (and answers, for solve). `409 {"ok":false,
  "in_flight":true,…}` while a run of the same mission is going;
  `400` for a malformed day; `404` when the mission was deleted. The run's
  end is in `GET /api/missions/<id>/runs` (`status`, `failure_count`) and
  the day's `scanned` flag in `days`. Scan, solve and dream are separate
  missions, so one of each can run at once.
- **Grants** — the scan agent has memory's tools and `Read`, no shell,
  and one grant: read on the candidates file. The session logs are read
  by the engine's own step, not by the agent. The solve holds the dream's
  READ over the home directory. Missions never prompt: a call outside the
  grants is refused.

## Memory is hosted by the engine

Since 2.3 (unreleased) there is no `ling-mem` process: `ling` opens the store
at start-up and serves it (`src/memory_host/`, code in `crates/linggen-memory`).

- **One writer.** The engine holds `memory/.store.lock` (flock) for its life;
  the file names its pid and port. A second opener is refused.
- **Doors.** `/mem/*` (memory's own routes) and `/mcp/memory` on the engine's
  port — the only port; the phone's `/api/memory/<verb>`. Who reaches which
  is below. Nothing listens on the old daemon's 9528.
- **No address to set.** The engine's own callers use the store it hosts
  (`memory_host::url`); `[agent].ling_mem_url` and `$LING_MEM_URL` are gone,
  and an old config's key is ignored.
- **The old binary.** `~/.local/bin/ling-mem` becomes a stand-in: data verbs
  run as `ling mem`, `status` says running at the engine's version and port,
  lifecycle verbs do nothing; a stand-in that differs from what this engine
  writes is rewritten. The real binary is kept as `ling-mem.prev`.
- **Takeover.** A running `ling-mem` named in `daemon.json` is stopped
  first; each engine version copies the store once to
  `memory/backups/takeover-<stamp>/` before it first opens it.
- **CLI.** `ling mem <verb>`: to the engine when it is up, else on the store
  directly under the lock.

The engine loads the embedding model and runs one throw-away inference in the
background. Startup, health, and calls that do not embed (list, get, upkeep
reads) do not wait. A call that embeds (add, add_batch, search) waits for that
inference up to 20 seconds (`embed::WARM_WAIT`, under the MCP door's 25), then
runs as usual; only a warm-up that outlasts the bound refuses the call, saying
how long memory has been warming. A refused write is neither applied nor
queued; retry once memory is warm. Automatic recall (the engine's and the
plugin hook's) sends `skip_if_warming` and skips the turn quietly at once — a
recall never holds a prompt. A replica sends its writes with a client that
waits out the primary's bound, so it never queues a write the primary applies.

### Who reaches memory, and whose rows they see

One rule, in one place (`memory_host::caller`), for every memory route on
the engine's port. Every paired device is one row in `paired-devices.json`
with a **kind**:

| Caller | Who it is | Sees |
|---|---|---|
| This machine (loopback; its own web UI over its channel) | the owner | all of the owner's memory |
| A `machine` — approved with Allow on this machine | the owner's other computer | all of the owner's memory |
| A `phone` — paired with the on-screen code | its own person | its person's rows only |
| No token, an unknown token | no one | nothing |

- **Whose rows** comes from the caller, never from the request body — unless
  the caller is the owner or a machine. Every memory call names its caller
  (`Who`: the owner, or one person); for a person, memory itself removes the
  body's own claims and stamps the person (`/api/memory/<verb>`, a skill
  page's memory calls). The engine's door decides which caller a request
  is; memory decides what that caller may reach.
- **Over the network** (through the LAN gate, with a token) a machine
  reaches `/mcp/memory` — for a machine with only a plugin — and the replica
  protocol under `/mem`: `/api/sync/changes`, `/api/sync/rows`,
  `/api/sync/role` and the writes a replica pushes. Asking to connect
  (`/mem/api/connect/request`, `/mem/api/connect/status/<id>`) needs no
  token. Everything else under `/mem` is this machine's. A phone's token is
  refused on all of these (403); its door is `/api/memory/<verb>`.
- **A phone that is another person** (not the owner's own phone: one whose
  claim of this machine's account is verified, `permission-spec.md` § A
  paired device's standing follows its account — a claim not verified is
  its device) runs only the verbs kept per person: `search`, `list`, `count`,
  `get`, `add`, `add_batch`, `update`, `delete`, `forget`,
  `session_start`. Any other verb — including one memory gains later — is
  refused (403 `FORBIDDEN`), and so is a body the stamp cannot mark (not a
  JSON object). The owner's own phone runs every verb but `restamp` and
  `accounts`.
- **Memory checks the person too.** A write stamped with a person
  (`account_id`) may name only that person's rows in `replace_ids`; any
  other id refuses the whole call before anything is written, with the
  same not-found answer whether the id is missing or someone else's. Its
  duplicate check (same table and the other table) looks only at that
  person's rows, so it never merges into, promotes over or deletes another
  person's row. `add_batch` stamps every row with its person. `update`
  checks the row's person before the user-voice guard reads it. The
  owner's writes (no `account_id`) deduplicate and replace as before.
- **Allow / Deny** happen only at this machine.
- **On by default**, for approved machines only.
- **Old rows** are classified once: a row the Allow ceremony recorded
  (`memory/connect-devices.json`) is a machine; any other is a phone.

## Across machines

One person, one memory across all their machines (designed 2026-10-21,
building). The stores form a star: one **primary**, any number of
**replicas**.

- **Setup** — every machine runs its own Linggen with its own store;
  clients talk only to the local one. One setting picks the primary:
  *local* (default) or another machine's Linggen (`host:port`, its engine
  port). Set it in Settings → Memory → Settings → Other machines. An engine never uses
  another machine's store by address; an old config that names one is
  ignored. A replica that saved a primary at the old daemon's `host:9528`
  reaches nothing there: it pairs again with `host:9527`.
- **Plugin only** — a machine with a plugin and no Linggen has no store: the
  plugin points at another machine's `/mcp/memory` with that machine's
  token (`/linggen:config`).
- **Joining** — a replica asks; the primary's machine shows Allow /
  Deny. Allow issues a device token the replica keeps, on a `machine`
  row. Only the primary's own machine can approve.
- **Reads** — from the local copy, synced from the primary; work
  offline.
- **Writes** — go to the primary; queued while it is away, sent when it
  returns. The primary wins a conflict and files a review item.
- **Dream and solve** — only on the primary's engine, one run per day
  (a lock). Inline fixes and scan work on every machine.
- **Status** — role, primary, online/away, queued writes: shown in the
  console, 9527 and `/linggen:config`.

Settings → Memory → Settings holds memory's two own knobs, the days a
short-term row is kept and the recall threshold (`GET`/`PUT
/api/ling-mem/config`, the owner only), above the primary setting.

A star needs one machine reliably online; multi-primary sync is in
Future.

## Rules

1. The user's voice changes only with the user — no silent rewrites
   of anything they said.
2. Record what was said; never invent details. Stamp ages to a date,
   not "now".
3. The file beats the memory — anything readable from the workspace
   stays out.
4. Curate, don't accumulate — value grows over time, not row count.
5. Merging the agent's notes is free; generalizing about the user is
   always done in front of them.
6. Never store secrets, at any layer.
7. Status rows are perishable — a capture that changes a subject's
   status supersedes the prior status row in the same write; the
   review queue is the backstop, never the plan.

## Evaluation

LongMemEval is a retrieval regression check only — it rewards
hoarding, the opposite of this design; never optimize toward it. The
real scorecard is the write-side eval (six axes: extraction, routing,
dedup, reconcile, decay, secrets), which drives each scenario through
a real engine against a throwaway store and judges the end state.

## Open / next

- Scope on Codex is read-side only: its hook runner cannot rewrite
  tool input yet, so recall is scoped but the model's own writes go
  unstamped there (no `source_session` either).
- Schema v2 (contexts/tags dropped; `hook` → `summary`, `cwd` →
  `scope`, values carried) runs only on `ling-mem apply-schema --yes`,
  after a backup to `~/.linggen/memory/backups/schema-v2-<ts>/`. Until
  then v1 stores keep working; the engine sends `scope_root` or
  `cwd_scope`, whichever the server declares.

## Future

- Cross-device sync — machines: see **Across machines** above; the
  phone's half is designed in `phone-memory-spec.md` (2026-09-08, not
  built).
- Multi-primary sync (an op log per store, merged by version vectors) —
  when no machine can be the primary.
- Soft-forget (archive): hidden from search but recoverable.

## Where the detail lives

Capture protocol: the engine system prompt + the skill's MCP
instructions · offline judgment, scan and solve: `agents/memory.md` ·
the session scan: `crates/linggen-memory/scan/` (`ling mem scan`) ·
dream/condense procedures on plugin hosts: the linggen skill's runbooks · store schema and
CLI: `linggen-memory` docs · layout: `storage-spec.md` · tool
dispatch: `tool-spec.md` · the phone's memory and the account stamp
on rows from a phone: `phone-memory-spec.md`.
