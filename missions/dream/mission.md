---
name: Memory dream (consolidation)
description: Nightly memory consolidation — promotes durable episodic memories into the long-term semantic store and forgets the rest. Engine-driven; runs even when no session is active.
schedule: "0 3 * * *"
enabled: true
allowed-tools: ["Bash"]
---

# Memory dream

This is Linggen's built-in **memory consolidation** pass — the "sleep"
half of the memory system.

While you work, the per-session encoder writes useful signal into the
short-term **episodic** store. This mission is the offline counterpart:
on a daily schedule (and as a catch-up the next time Linggen runs after
a missed night), the engine reviews episodic memories that have aged
past their TTL and makes one terminal decision per row —

- **promote** durable user biography, cross-project preferences,
  decisions-with-reasoning, and reusable gotchas into the long-term
  **semantic** store, then
- **forget** (evict) the rest.

The worklist and TTL policy are engine-owned (the consolidator never
queries or decides retention itself). Promotion only ever *adds* to the
semantic store with an optional supersedes link — it never destructively
rewrites or deletes existing long-term memory; that stays
user-initiated.

**You can stop a run** from the mission UI if it ever misbehaves — it
simply re-arms on the next cycle. **Deleting this mission** disables
automatic long-term memory curation: episodic memories will still be
captured and recalled, but they will age out without being promoted.
That is a supported choice, not a bug — nothing is lost that the binary
can't still hold; only the automatic curation stops.
