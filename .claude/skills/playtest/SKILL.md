---
name: playtest
description: Play 灵境 (Lingjing) by its play bot and review what it saw — headless, muted, scratch saves in a scratch install, never Hanli's save or his Chrome. Use before any release (the release gate runs it and waits on your review), after a deploy batch of skills/lingjing changes, or whenever Hanli asks to test 灵境 / 测一下灵境. `playtest --selftest` proves the bot still catches its seeded bugs.
---

# playtest — 灵境's play bot, and the review it waits on

The bot lives in the skills repo: `skills/lingjing/tools/playbot/` (`run.sh`, `playbot.mjs`, `lint.mjs`,
`serve.mjs`, `report.mjs`, `seeded-bugs.json`). It serves a skills checkout the way the engine serves it, runs the
page's Verb door as `node rules.mjs` against a scratch data folder, and drives the real page with Playwright at
430×932, 375×812 and a Mac width: N 降妖 fights, a story fight, a 切磋 bout, and a walk around a kept fight (储物袋,
饭桶, a road, a reload mid-fight and back). Moves are random among what the page offers as legal, seeded.

Never point it at `~/.linggen/skills/lingjing/data`, never `--headed` on his screen, never his Chrome profile.

## Steps

1. **Run it** on what you are testing, from the engine repo:
   - `just playbot` — origin/main of the skills; `just playbot --ref <sha>` — a commit; `just playbot --install <lingjing dir>` — an install played read-only (its data never touched).
   - `--sizes 430x932,375x812,1280x820` and `--fights N` (降妖 per size) narrow or widen it. It takes minutes; it is not part of check.sh.
   - The report lands in `skills/lingjing/report/<run-id>/` (or `--out`). Exit 1 = hard findings.
2. **Read** `findings.json` (every hard finding names its screenshot: no-response, no-feedback, mana-waste, no-spoils, frozen-by-fight, resume-mismatch, console-error, layout), `refusals.json` (every refusal the rules gave, with its reason — read them as a player would: is each one fair?) and `played.json` (what was played and what could not be).
3. **Look at every screenshot** — `contact.png` first for the whole run, then each file under `shots/` with the Read tool — and answer `checklist.md`'s four questions for each one (references/review.md says what a good answer looks like).
4. **Write `review.json`** in that report folder, keeping its `run_id`:
   `{"run_id": "<as written>", "verdict": "pass" | "fail", "reviewer": "<you>", "shots_total": N, "shots_answered": N, "notes": [{"shot": "shots/…png", "answer": "…"}]}`.
   A hard finding is a fail. Pass only when every screenshot was looked at (`shots_answered` = `shots_total`). A review for another run id does not count anywhere.
   In a release gate run, then: `just release-gate --verdict <gate run dir>` — the gate stays REVIEW PENDING until then.
5. **Report** to the user: what was played (each fight and its outcome, at which sizes), what failed (finding → screenshot → the likely cause in one line), what could not be played and why, the refusals worth a person's eye, and the verdict. Say plainly what was seen in screenshots versus what the checks inferred.

## Selftest — the seeded bugs

`just playbot --selftest` replays `seeded-bugs.json`: each a skills ref (or a ref + a patch under `seeded/`) that
holds one of the bugs of 2026-11-04/05, and the finding the bot must raise for it:

| id | where | must raise |
|---|---|---|
| kept-fight-freezes-fantong | 48f0b83c (before e20846c1) | `frozen-by-fight` |
| hand-overlaps-at-six | 48f0b83c (before 4b14bd09) | `layout` · covered · a card / cost |
| yangzhen-on-empty-rank | 48f0b83c | `mana-waste` |
| win-without-spoils | 4b14bd09 + seeded/no-spoils.patch | `no-spoils` |

A seeded bug it misses (exit 3) means **the bot is broken** — fix the bot before trusting any run of it. When a new
bug slips past it, add it here: the ref before its fix, and the finding that should have caught it.

## The release gate

`linggen/scripts/gate/gate.sh` (`just release-gate`) runs this on the skills Linggen.app vendors and the ones the
phone pack is built from (`GATE_PHONE_SKILLS_REF`, else the same), after building the phone's 灵境 assets from them
and running its `lingjing_*` tests (a skipped one is a GAP). A hard finding FAILs; the review shows REVIEW PENDING
until this skill's step 4 is done for that run id; `--skip-playbot` is a GAP. `doc/test-design.md` § 4.

## Phone (later)

Maestro MCP on the iOS simulator will drive the phone's own native screens (the 所得 sheet, the 储物袋, the board host)
the same way — placeholder: not wired yet. Until then the bot plays the page the phone's boards share; the native
screens on a real device still need the tester (Hanli or the deployer with the phone in hand).
