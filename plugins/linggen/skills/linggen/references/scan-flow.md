# Scan flow — stage one day's session logs (`/linggen scan <YYYY-MM-DD>`)

Backfill staging: always user-triggered, idempotent. A scan only stages
episodic rows; the dream judges them later (`dream-flow.md`). Read this
file when scanning, together with `extractor-prompt.md` (what to stage)
and `routing-rules.md` (where it goes).

Calls are written as `<tool> {arguments}` for the memory MCP tools; the
`ling mem` CLI form follows where one exists (CLI only when Linggen runs
on this machine).

## Before the scan

- **Primary only.** `memory_upkeep {"op":"role"}` first. `role: replica`
  → reply exactly `Memory is dreamed on the primary (<upstream>)` and
  stop: the primary's sessions are scanned there.
- Today and future dates are not scannable — say so and stop.

## Steps

1. Run `Bash ling mem scan <date>` (zero-LLM session walk →
   `~/.linggen/memory/.scan-output.jsonl`; the command is part of
   Linggen, so no path to resolve). In Linggen itself a day can also
   be scanned from Settings → Memory.
2. **Skip covered sessions.** List the day's existing rows —
   `memory_search {"op":"list","tier":"episodic","day":"<date>"}` (page
   with `offset`; promoted twins may live in semantic, so list that tier
   for the day too) — and collect their `source_session` ids. Drop every
   scanned session already in that set — live capture or a prior scan
   contributed it. This is what makes re-scanning safe on
   partially-captured days.
3. Judge the remaining candidates per `extractor-prompt.md` +
   `routing-rules.md`; write keepers to episodic with `memory_add` (no
   `tier`), `occurred_at` set to the session time and `source_session`
   the scanned session's id. What the user said in their own words is
   `from=user`; never pass `user_directed` in a scan.
4. **Stamp scanned** — `memory_upkeep {"op":"harvest_day","date":"<date>"}`
   (CLI: `ling mem harvest-day <date>`). This does **not** mark the day
   remembered: new rows clear its `dreamed` flag, and the next dream
   (nightly or the day's dream button) judges them. Nothing new staged →
   still stamp, report `CLEAN`.

## An incomplete scan is not stamped

Stamp only after every scanned session was judged. When `ling mem scan`
failed, its output could not be read, or the run stopped before the last
candidate, write what you have, do **not** stamp, and say which sessions
are left — the day stays unscanned and the next scan picks it up (step 2
skips what was already written).

## Reporting

One line per keeper (`STAGE <id> "<gist>"`), then one totals sentence:
sessions seen, skipped as covered, rows staged, and whether the day was
stamped.
