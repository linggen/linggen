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
the memory system. Engine-internal: no `shared-memory` skill required,
no session-file scanning, no transcript ingestion. Only consumes what
the per-session encoder already wrote into episodic.

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
- **Schedule** lives here (`schedule: "0 3 * * *"` — 03:00 daily) — edit
  to taste, or set `enabled: false` to pause.

## Turning it off

Three off-switches, in increasing order of finality:

- **Stop a run** from the mission UI if it ever misbehaves — the
  schedule keeps running, the next cycle re-arms.
- **Pause** by flipping `enabled: false` (or the toggle in the mission
  UI). Schedule preserved; nothing fires until you re-enable.
- **Delete the mission** to permanently disable automatic long-term
  memory curation. Episodic memories will still be captured and
  recalled — they'll just age out without being promoted. Supported
  choice, not a bug; nothing is lost the daemon can't still hold. The
  install sentinel (`~/.linggen/missions/.builtin-missions-installed`)
  prevents future daemon starts from re-seeding it.
