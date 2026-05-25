---
name: dream
description: Nightly memory consolidation. Promotes durable episodic memories into the long-term semantic store and forgets the rest. Built-in — no external skill needed.
schedule: "0 3 * * *"
enabled: true
# The engine special-cases this mission by id (`DREAM_MISSION_ID = "dream"`)
# and runs `scheduler::dispatch_dream_mission` directly instead of the
# generic mission-agent loop. The engine selects the past-TTL worklist
# itself, then spawns a `ling-mem-autonomous` subagent (no AskUser —
# unattended at 3am) for the promote/delete phase. So this list is
# empty by design.
allowed-tools: []
permission:
  paths:
    - path: ~/.linggen/memory
      mode: edit
  warning: >-
    Writes to ~/.linggen/memory via the local `ling-mem` daemon
    (promote episodic → semantic, then evict past-TTL rows). Does not
    touch project files.
---

# dream

Linggen's built-in **memory consolidation** pass — the "sleep" half of
the memory system. Engine-internal: no session-file scanning, no
transcript ingestion. Only consumes what the per-session encoder
already wrote into episodic.

## What it does

On a daily schedule (and as a catch-up the next time Linggen runs after
a missed night), the engine reviews episodic memories that have aged
past their TTL and makes one terminal decision per row:

- **Promote** durable user biography, cross-project preferences,
  decisions-with-reasoning, and reusable gotchas into the long-term
  **semantic** store.
- **Forget** (evict) the rest.

The autonomous consolidator also deduplicates **silently when
confident** — if an episodic row clearly paraphrases an existing
semantic row, it skips the promote and just deletes the source.
**When confidence is low** (contradictions, partial overlap, "related
but not identical"), it does NOT pick a winner, merge, or rewrite the
existing row. Those resolve in a live-recall pass with the user
present — never as a silent offline rewrite. The dream is hippocampus,
not editor.

## Where the knobs live

- **TTL** is owned by `ling-mem`, not the engine. Each run fetches
  `episodic_ttl_days` from the local daemon's `/api/config` (typically
  `http://127.0.0.1:9888`; configurable in **Settings → General →
  Ling-mem URL**). Change the TTL in the ling-mem console and the next
  dream run honors it.
- **Schedule** lives here (`schedule: "0 3 * * *"` — 03:00 daily). Edit
  to taste, or use the Mission UI's pause toggle / stop / delete
  controls if you want the consolidation off temporarily or for good.

## How it runs

The engine special-cases this mission and drives it from
`scheduler::dispatch_dream_mission`. One run is:

1. **Trigger** — daily 03:00 cron, or the post-turn catch-up if the
   last successful run is older than `dream_catchup_hours` (engine
   config, default 24). A `dream_running` flag guards against
   overlapping runs; a concurrent trigger records a `skipped` entry
   and bows out.
2. **Open session** — create a mission-scoped session, emit
   `MissionTriggered`, persist the kickoff line. All subsequent
   subagent activity hangs off this session for audit.
3. **Fetch TTL** — `GET <ling_mem_url>/api/config` →
   `episodic_ttl_days`. On any error (daemon down, non-200, malformed
   JSON) fall back to the engine's baked-in default. The resolved
   number drives the cutoff: `cutoff = now − episodic_ttl_days`.
4. **Build worklist** — `ling-mem list --episodic --format json`,
   then keep only rows whose `COALESCE(updated_at, created_at)` is
   strictly older than the cutoff. The engine, not the binary, owns
   the TTL policy; the binary stays policy-free.
5. **Empty worklist?** Skip the LLM phase entirely, record
   `promoted=0 deleted=0`, finish. Cheap exit — most runs.
6. **LLM phase** — spawn the `ling-mem-autonomous` subagent
   (`agents/ling-mem-autonomous.md`; tools = `Memory_query`,
   `Memory_write`; **no AskUser**) with the worklist + cutoff as the
   task. For each row, the subagent decides:
   - **Promote** — `Memory_write({verb: "add", host: "linggen",
     content, type, from, contexts, tier: "semantic"|"core"})`, then
     `Memory_write({verb: "delete", tier: "episodic", id})`.
   - **Silent dedup** — if `Memory_query({verb: "search"})` returns a
     semantic row clearly meaning the same thing, skip the add and
     just delete the episodic source.
   - **Skip-on-uncertainty** — for related-but-not-identical pairs,
     partial overlaps, or contradictions, do nothing in this pass.
     Append-only is the floor; the conflict resolves in a live recall
     with the user present.
   - **Delete** — for not-worth-keeping rows (pure activity,
     re-derivable, single-mention noise): `Memory_write({verb:
     "delete", tier: "episodic", id})`.
7. **Deterministic backstop** — `ling-mem evict --before <cutoff>`
   sweeps any past-TTL row the subagent didn't terminally handle.
   Failures here are non-fatal — the rows simply age out on the
   next run.
8. **Report** — the subagent emits one terminal line:
   `CONSOLIDATED promoted=<n> deleted=<n>`. The engine parses it,
   writes a `MissionRunEntry` with status (`completed` on success,
   `failed` on parse/subagent error), logs `dream mission:
   consolidate ok (promoted=N deleted=N)` at INFO. The Mission UI
   surfaces the entry under run history.
9. **Release the overlap guard** — `dream_running = false`; the next
   trigger is free to fire.
