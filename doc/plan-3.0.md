# Linggen 3.0 — plan

**Status: plan, work started 2026-11-15** (Hanli: 「先开吧」). Agreed in talk, Hanli
and Claude, 2026-11-15. Claude plans and keeps the whole in hand; the work itself
goes to Linggen agents and Claude subagents. It replaces `merge-and-slim-draft.md` and `dynamic-session-draft.md` (kept for
the measurements and the talk that led here).

Every number is tagged **(measured)** — taken on his Mac on 2026-11-15 — or
**(estimate)**.

## 1. What 3.0 is

- **One program.** `ling-mem` goes into `ling`: one binary, one process, one
  port (9527), everything always on. The name `ling-mem` is retired; there is
  only Linggen.
- **Smaller.** The game's art leaves the package; the Mac app and the iPhone
  app slim together.
- **One Ling, one Yinyue.** Each is one person, the same in every app and every
  session, and both see one world.

## 2. Milestones

| Release | Holds | Done when |
|---|---|---|
| **2.3** | Structure: architecture review, build speed, the merge, the engine split into modules, library upgrades, app slimming | Gate green on first install and on 2.2.1 → 2.3 (the takeover of an existing memory store); one process serving memory to Claude Code, Codex and OpenClaw seen live on his Mac; sizes re-measured |
| **2.4** | Behaviour: one Ling and one Yinyue (§5) | Gate green; the scenario tests pass; his three scenarios seen live |
| **3.0** | A release of its own, after 2.4 and likely a 2.5 of fixes | Hanli's call, once everything has settled |

## 3. Rules for the work

1. **Few general tools; the agent judges.** Hanli: 「我只是举了3个例子, 这样的
   例子有很多, 不能都通过工具或者代码来做. 工具要底层通用, 数量尽量少, 尽量让agent
   去做.」 A scenario is never a spec for a feature: no tool, coded condition
   or computed field per scenario.
2. **A move and a change of behaviour never share a commit.** Splitting moves
   code; it changes nothing a user or a test can see.
3. **Everything lands on main as it goes**, tests green; no long branch.
4. **Each release passes the gate** (`release-checklist.md`).

## 4. 2.3 — structure

"No change in behaviour" is true of the **engine**. The app is another matter:
slimming needs new skills commits, so 2.3's app carries everything on skills
main since 2.2.0's game bytes (pill and seal numbers, names, 问签, the Mac hand,
切磋 name plates, the cfo Watch fix).

### Step 0 — architecture review (read-only, written up for Hanli)

- Module boundaries and who may depend on whom; the target layout for step 3.
- What the merge lets us delete: two trust systems, two updaters, two telemetry
  modules, two MCP doors, and the engine's client code for the memory daemon.
- How skill, tools, working directory and permission are bound to a session
  today, and how they come apart — the ground 2.4 stands on.
- Oversized files and functions only where the work touches them.

### Step 1 — build speed (build config only)

| | Today | With the change |
|---|---|---|
| One file edited, rebuild | 56 s (measured) | 8–18 s (measured, release profile + incremental) |
| Clean build | 144 s (measured) | 161–165 s (measured) |

- A dev profile with incremental compilation on; shipping builds keep
  strip + thin LTO + one codegen unit.
- One build cache shared by all worktrees, so a fresh worktree compiles only
  our own code. Cost: one build at a time.
- `cargo check` where the question is only "does it compile".

### Step 2 — the merge

Decided: one binary, one process, always on (web UI, Ling, memory); the app
stops passing the idle-shutdown flag it passes today.

What it touches (files naming `ling-mem` or 9528, counted 2026-11-15):
engine `src` 30 · engine `scripts` 9 (the gate among them) · engine `doc` 21 ·
`linggen-memory/plugins` 116 (Claude Code, Codex, OpenClaw) · `skills` 55 ·
`linggen-app` 53 · `linggensite` 18 (install scripts, guide) · `linggen-vscode`
18 · `linggen-mobile` 8 (no direct use of 9528 found; confirm in step 0).

Cost of the merge, accepted:

| | Measured |
|---|---|
| Binary, merged, shipping shape | 220 MB (133 + 109 = 242 apart) |
| One file edited, rebuild | +11 s (56 → 67 s, before step 1's profile) |
| Clean build | ≈ 480 s against 144 s (machine was loaded; slow side) |
| Process memory | 279 MB + 304 MB today, one process after |

- Restarting the engine takes memory away from every host for a few seconds:
  the plugin hooks stay silent on failure and retry once.
- One crash takes both. Memory's heavy work (embedding, compaction) runs off
  the async runtime, with a memory limit.
- Dependencies are locked at today's versions for this step: a fresh resolve
  already broke the trial build once (fastembed 5.17 needs candle 0.11).

Takeover and rollback:

- First start of 2.3 backs up the memory store (≈ 70 MB today), stops any
  running `ling-mem`, and opens the same store in place — no data migration.
- The gate's upgrade path becomes 2.2.1 + ling-mem 1.13 → 2.3; its
  ling-mem-specific checks are rewritten.

### Step 3 — the engine in modules

Memory is the first module. The rest follow the review, one commit per split,
moves only.

### Step 4 — library upgrades

1. Patch and minor versions in one batch.
2. Majors by group, each verified alone: compute + embedding (candle 0.11 with
   fastembed); the database group (lancedb, arrow, datafusion, kept aligned) —
   with the memory benchmark and proof that an old store still opens; network
   and crypto.
3. Unused features of the database libraries switched off. They are about 60 %
   of the program's code (measured; the SQL parser alone is 21 MB before
   link-time optimisation). Saving: unknown until tried.

### Slimming — in parallel (skills, mobile, site)

灵境 stays a local game: page, rules and saves on the device, playable offline.
Only its art moves to Cloudflare, one copy for Mac and phone.

| | Today (measured) | After (estimate) |
|---|---|---|
| Mac app installed | 359 MB (+ 109 MB ling-mem apart) | 240–255 MB |
| Mac download (dmg) | 245 MB | — |
| ↳ 灵境 in the app | 209 MB (art ≈ 146, code 43, tools + tests 15) | 75–90 MB |
| ↳ `ling` | 132 MB | ≈ 154 MB merged, without Yinyue's 66 MB |
| iPhone upload (ipa) | 126 MB | 50–60 MB |
| iPhone installed | 169 MB (灵境 106) | — |

- **In the package:** the first chapter's art and the art every chapter
  shares — ≈ 45 MB (estimate, a lower bound). Proved by running the play bot
  through 序章 and 第一章 with only that art present and downloads blocked.
- **The rest downloads:** Mac — all of it, in the background, right after
  first launch. Phone on Wi-Fi — the same. Phone on cellular — only the chapter
  that comes next; the rest waits for Wi-Fi.
- **Never packaged:** 灵境's `tools` and `tests` (15 MB, measured).
- **Yinyue** (66 MB measured: model 16, animations 50): compress first —
  animations to a compact format, compressed textures — measure, then decide
  whether she is hosted or simply stays in the package. Meeting her never
  waits on a download.
- **Sources.** Our own files on linggen.dev's Cloudflare storage, each checked
  by content hash. Models: Hugging Face first, then the China mirror, then a
  copy on our storage for every model whose licence allows it; all pinned by
  hash. No China mirror for art yet: when Cloudflare cannot be reached the
  game plays on with pictures missing and retries.

## 5. 2.4 — one Ling, one Yinyue

Hanli: Ling is one person and Yinyue is one person, wherever they appear —
no "Yinyue in DJ" who differs from "Yinyue in 灵境". The unity comes from one
prompt and one injected world, not from moving transcripts around.

**Sessions.** Linggen makes one session a day, shared by all apps. Sessions
the user makes are unlimited and stay apart; they are given the same world.

**What code provides — and nothing about any scenario:**

| | What | Today |
|---|---|---|
| Now | World state: one line per app (declared by the skill), the machine, the body; in every session | Built; app-declared lines designed 2026-11-13, not built |
| Past | One tool to read the activity log, filtered by time and app, returning totals | Built; totals to add — the model must read a duration, never add one up |
| Memory | Search and write | Built |
| Notes | A small note of her own, in front of her next time | New |
| Alarm | "Look again at / after" that she sets herself | New |
| Voice and hands | Her words reach the user wherever they are; app actions the skills declare | Built |

Everything else is theirs: what to look up, what to connect, whether it is
worth a sentence, how to say it, whether to come back to it.

**When she looks** is already built and stays: a glance every 30–50 minutes,
mostly silent; facts an app tells her, held until the user has gone quiet;
system events. The coded machine conditions that exist stay as doorbells; no
new ones are added.

**The daily session becomes the one session.** Yinyue's daily thread today
seats her alone. Three changes:

1. Ling is seated too; every app's chat panel is this session. The per-app
   daily sessions on the Mac go away.
2. Skill prompt, tools, working directory and permission are chosen per turn
   by the page the user is on. This is the one heavy change.
3. Her special case for speaking inside an app's own session is removed —
   there is one place her words go.

The session list shows one Linggen row a day. An app's page shows the day's
conversation with other apps' stretches folded to a line. No folding of the
prompt, summary cache or thread-recall tool: one day of app talk on his Mac is
a few thousand tokens (measured over three weeks), and the session already
rolls to a fresh segment near its context limit.

**Guardrails, in code:** a ceiling on unprompted lines per hour and quiet
hours; a quiet moment may speak and ask, never act; privacy lines (holdings
are never told to her).

**Scenario tests** are how it gets smarter without more code: a world state
and a memory in, "should she speak, and naming what" out; rerun on every model
or persona change. The first three are Hanli's — work 8 h, play 2 h and a low
HRV in 灵境; disk under 50 GB and a 12 h 闭关 during a coding session; something
he told her he cares about, set against how the day has gone.

## 6. Before a step can ship

| Check | Blocks |
|---|---|
| Mixamo's terms on offering animation files | Hosting Yinyue's animations |
| The VRoid model's redistribution licence (open since 2026-08) | Hosting Yinyue's model |
| The picture model's licence, and a steadier source than a personal account's conversion | Mirroring it on our storage |
| Cloudflare storage price (≈ 10 GB of models, plus art) | Going live with the mirrors |

## 7. Claude's calls — say no to any

- Memory's code comes into the engine repo with its history.
- The command line becomes `ling mem …`; the version is the engine's alone.
- No forwarding from 9528: the three plugins and the install scripts move to
  9527 in the same release (no real users yet).
- A rollback from 2.3 to 2.2.1 is the old binaries plus the first-start
  backup: memories written under 2.3 are lost. The old `ling-mem` binary is
  kept on disk, not running, for that one release.
- How the one session is stored — one file or several — and how each turn is
  assembled.

## 8. Open

- Where the art's source of truth lives once it has left the package, and how
  an art update ships.
- What the phone gets in each milestone; its App Store train is separate.
- How the world state reaches a session Linggen does not host (Claude Code or
  Codex in a terminal) — through the plugin's hook, or not at all.
- Whether the gate tests a first launch with no network.
