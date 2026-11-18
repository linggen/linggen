# Solve runbook — `/linggen solve`

Read this when solving the review queue (`/linggen solve`, or a user
asking to clear it); the skill's Solve section points here.

A dream settles review items itself from evidence and queues only
what needs the user — so the open queue is the rare leftover: a
user-voice row that would change, rows the user must pick between
(`contradiction`), or a `chain` / `stale-status` / `subject` item whose
evidence the dream could not read (refused, or a project not on that
machine — the dream looks at code, docs and git history read-only). Items the dream settled are already
closed, with its why. Scope and index fixes are not queued — the dream
applies them itself. Memory only bookkeeps; **you are the solver**,
with this session's model, tools, and user.

0. **Primary only.** `memory_upkeep` op=role first: on a replica, say
   `Memory is dreamed on the primary (<upstream>)` and stop — review
   items are solved on the primary.
1. **Back up, then list.** `ling mem export` first (on the machine that runs Linggen; it works with Linggen running) (one snapshot per
   solve session), then `ling mem issues --format json` (or the
   `memory_upkeep` op=issues MCP call). Empty → say so, done.
   **Work the whole queue in this run.** Every item ends solved,
   dismissed, or left open with its reason. Go item by item without
   pausing: don't stop early to report partial progress or ask
   "continue?" — only the final report summarizes. If the context gets
   heavy, finish the current item, report the counts and how many
   remain, and say the next `/linggen solve` continues from there.
   **Main chat only:** solve never uses Task or subagents. This run
   gathers every item's evidence itself (`memory_search` op=get, Read,
   git via Bash) and is the only writer (`memory_add`,
   `memory_upkeep` op=issue_resolve).
2. **Per item, gather evidence at solve time — solve it yourself
   first.** Fetch the rows (`memory_search {op:"get", id}`, CLI
   `ling mem get <row_id>`), then read
   whatever settles the question: git history
   (`git log --oneline --since=<row date>` in the named repo), the
   code, docs, files. The row was written before the world moved;
   your evidence decides what's true now. If this session lacks a
   tool to check something, leave that item open with the reason and
   include it in the end-of-run question. Asking the user is the
   last resort, not a step. **Evidence you can't read** — a
   permission prompt was declined, or the path is outside your
   grants: don't ask again and don't try it with another tool. Leave
   that item open, move on, and at the end ask the user ONE plain
   question covering all of them (e.g. "The <project> items — want me
   to check them?"). On a yes, go read.
3. **Apply the confidence rule.** Evidence settles it AND every
   affected row is your own note (`from=derived`) → solve directly, no
   ask: one `memory_add` with `replace_ids` (CLI: `add --replace <id>`)
   writing current truth. Ask the user ONLY when evidence cannot
   settle it after a real attempt, or a user-voice row (`from=user`)
   is affected. When you do ask: **ONE item per question**, phrased
   as a simple fact question in plain words — one-line gist per fact,
   then "same thing, or different?" / "which is true now?", with your
   recommendation (AskUserQuestion on Claude Code; plain numbered
   options elsewhere). No row ids, commit hashes, or cluster jargon
   in the question. User-voice fixes carry `user_directed:true` after
   their answer.
4. **`subject` items.** Rule on coherence yourself from the full
   member contents: one genuine subject → digest per the condense
   drafting rules (`add --replace <id>` per member),
   resolve `resolved`; distinct workstreams → resolve `dismissed` —
   the dismissal IS the ruling; the detector never serves that
   cluster again. Ask only when you genuinely can't tell.
5. **Close as you go.** After each item:
   `ling mem issue-resolve <id> --outcome resolved --note "<what you did>"`
   (or `memory_upkeep` op=issue_resolve). Not worth fixing → `--outcome dismissed`.
6. **Report once, at the end** — one line per item: `SOLVED <id> <what
   changed>` / `DISMISSED <id> <why>` / `SKIPPED <id> needs <dir>` — then
   the counts (solved / dismissed / left open) and the one plain question
   for the items whose evidence you couldn't read.
