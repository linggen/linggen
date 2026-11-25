# Linggen plan — 2.3, 2.4, 2.5 (begun as the 3.0 plan)

**Status: plan, work started 2026-11-15** (Hanli: 「先开吧」). Agreed in talk, Hanli
and Claude, 2026-11-15. Claude plans and keeps the whole in hand; the work itself
goes to Linggen agents and Claude subagents. It replaces `merge-and-slim-draft.md` and `dynamic-session-draft.md` (kept for
the measurements and the talk that led here).

**Version numbers, changed 2026-11-18** (Hanli: 「2.4 要发布一版, 2.5 也要等安度发. 3.0
是暂定的, 可能会到2.9以后才发, 到时候再说. 3.0是长期目标」): the work this plan
calls 2.4 — everything in §5, which for two days (2026-11-16/17) was to ship as
"3.0" — ships as **2.4**. **2.5** follows on its own schedule (first item: one
MCP door). **3.0** is a long-term goal, perhaps after 2.9, half a year to a
year out; what it holds is decided then. The file keeps its name.

**3.0 decided, 2026-11-24** (Hanli: 「memoryV2可以作为linggen 3.0发布了, 和lingyu一起」,
「中间可以发2.7, 2.8, 如果有必要」): 3.0 is no longer far off. It is **Memory V2 + 灵语
(Lingyu)**. 2.7 and 2.8 go out before it only if needed; their unreleased work is
folded into the 3.0.0 CHANGELOG. See §0 below.

Every number is tagged **(measured)** — taken on his Mac on 2026-11-15 — or
**(estimate)**.

## 0. 3.0 — Memory V2 and Lingyu

**Why.** Linggen grows up with the person: years later it still knows what they
did and how far they came, and every agent they use shares one memory
(product-spec.md, principle 0). Delegating tasks is a commodity; shared, lasting
memory is not.

**What ships.**

| Part | What the person gets | State |
|---|---|---|
| Memory V2 | Notes in plain Markdown under `~/.linggen/memory/`, one writer, an index instead of vector recall; Claude Code, Codex and Linggen read the same notes; body, mind, people and moments are fenced from coding agents | built |
| V2 by default | Upgrading moves V1 rows into notes once, by rule, with no model call; `[memory] linggen = "v1"` stays the way back | built |
| Chronicle | A monthly "what you did and how far you came", written on the Mac when idle, editable | built |
| Yinyue's day | On this day in past years, recurring days, topics marked "don't bring this up" | built |
| Settings → Notes | Notes by source; void, restore, delete | built |
| Phone | The owner's phone memory goes into V2 | built (engine side); untried on a device |
| Lingyu | Spoken English lessons with Ling as teacher, live voice | built on its own branch; waits on Hanli (below) |
| Analytics | Where a new install came from (Apple Ads keyword or organic) and what it did after | built on phone and site branches; ships with the next phone build |

**Checked.** The unit, system, UI and end-to-end suites pass. The release gate
on a local 3.0.0 build passes: a fresh install lands on V2, an upgrade from 2.6.1
moves V1 rows into notes, rollback and forward keep every row, and an explicit
v1 stays v1. Not yet run: the gate on a signed draft, the 灵境 play bot, the 2.2.1
takeover baseline, or the app-bundle legs.

**Waiting on Hanli.**
1. Release from main (merge `refactor/53` first) or from the branch.
2. His yes to cut the draft, run the full gate, and publish, and whether the phone and the Mac ship on the same day.
3. Lingyu: whether 3.0 announces it, and the five owner questions in its design (its own app or inside Linggen, which market, cloud cost, voice, whether the synced record is in the first release).
4. The migration rows marked unsure (the builder-name conflict, two superseded rules, sensitive rows).
5. Turning on Sign in with Apple in the Apple developer portal, deploying the site, and the phone's App Store release.

**Not in 3.0.** Phone-side entry-by-entry sync (the Mac answers the phone's
existing verbs instead); the chronicle reading other agents' session logs;
anyone's memory besides the owner's on a shared Mac (2.8's per-person sharing
stays as built).

## 1. What this work is

- **One program** (2.3, published). `ling-mem` goes into `ling`: one binary,
  one process, one port (9527), everything always on. The name `ling-mem` is
  retired; there is only Linggen.
- **Smaller** (2.3 on the Mac, published; the phone with 2.4). The game's art
  leaves the package; the Mac app and the iPhone app slim together.
- **One Ling, one Yinyue** (2.4). Each is one person, the same in every app
  and every session, and both see one world.

## Progress

| Step | State (2026-11-17) |
|---|---|
| 0 Architecture review | Done — `architecture-review.md` |
| 1 Build speed | Done — `fast` profile, `scripts/worktree-setup.sh` |
| 2 The merge | Done, published in 2.3.0: memory runs inside the engine; an old daemon is taken over at start-up; `ling-mem` is a stand-in; one port — nothing listens on 9528. The plugins (`linggen@linggen` for Claude Code and Codex), the install scripts and the gate moved with it. Who reaches memory from another machine is built (approved machines only; a phone sees its own person's rows — `memory-spec.md`). Open: a run on a real second machine |
| 3 Modules | 3.2 done — `ling` (server, cli) and `crates/linggen-engine`. The further cuts (base, model, store, skills) are put off until 2.4 has landed, or dropped (decided 2026-11-16): they move files wholesale, would collide with the 2.4 branch, and with incremental builds a one-file rebuild is already 5–10 s — they would buy tidiness, not time |
| 4 Upgrades | Done, published in **2.3.1** (2026-11-17). Patch and minor batch in 2.3.0 (329 crates). The two major groups in 2.3.1, a release of their own because a database upgrade may change the store on disk: fastembed 5.17.4 on candle 0.11.0 (vectors bit-equal to 2.3.0's); lancedb 0.41 with lance 14.0.2 and datafusion 54 (a 2.3.0 store opens in place with every row; 2.3.0 still opens it afterwards — the gate now checks this upgrade and the way back on every release; new tables pinned to Lance file format 2.1). Cost (measured): `ling` 312.6 → 373.0 MB, the Mac download 219.5 → 238.1 MB. Open: the database libraries' unused features (step 4.3) to win that back; the memory benchmark (`evals/memory`) still speaks to the old two-program setup and was not run — identical vectors and identical search hits stood in for it |
| Slimming | Mac: done, published in 2.3.0 — Yinyue's clips baked; packaged skills leave out tools, tests and unloaded source art; 灵境's later-chapter art is fetched after install by content hash (133 files, held within seconds of first launch in the gate). Phone: built on branches, not merged — 灵境 in the app 106.7 → 86.1 MB (measured; the careful count), later chapters fetched by hash, everything on Wi-Fi and only the next chapter on cellular. Open: merging it, putting the phone's art set on the storage, a build on a device |
| 2.4 | On main since 2026-11-17 (engine, skills and phone), checks green. The activity log, the world block, notes and look-again, the guardrails, a turn's place; and the daily session, `agent.daily_session`, on by default since 2026-11-18: app chats, her moments and phone hand-offs in the daily session from 04:00, each turn in its page's place, other apps' stretches folded on a page. Reshaped on 2026-11-16/17 after review: a private app keeps a session of its own (one rule asked by every way into the day); a day is a person's, the person resolved from the device as memory resolves it; her stage and voice are held per person. The phone's side is on main too (hand-offs join the day; later chapters' art fetched by hash). Seen live once on his Mac with the setting on. Open: the setting on by default; the three scenarios seen live; the phone's art set on the storage and a build on a device |
| Kinds of session (§5.1) | Designed and agreed 2026-11-17, on main the same day: every kind of session is described by one spec and applied in one place (`engine/session_spec/`), locked by a test; the `code` kind of work (one prompt section, a few lines per model family); the three ways to ask for it (creating a session, changing one, `agent_run` — on by default with a folder to write); the Coding switch in the web UI. A delegated run is seated by its session's own spec, so the run itself carries the section (a system test reads what the model was sent). `evals/code` is on main: sixteen small tasks with hidden checks, three ways (with the section, without, a Claude Code session), a dry run that shows every check pass and fail. First comparison, 2026-11-17, three models: the section does not change how often the work is right (the same total with it and without); with it the changes are a little smaller, tests are touched less, and the report has its three parts. Open: one task to rework; long work in a real repository; the switch seen in a browser |
| A paired device's standing (§5.2) | Decided and designed 2026-11-17; merged to main the same day (engine; the phone's and the relay's sides on their own mains): the relay vouches for a phone's account, one standing (person and level) at every check point, a class for every route with a lock test, skill tools judged by the caller's level, the level kept on the device's row and set only at this Mac, the contract number; one type for who a person is, a session and its events given to their person, a turn under a ceiling. Open: seen live with a second account's phone; a phone on a Mac with no account |
| Review and refactor before 2.4 (§5.3) | Engine and phone reviewed 2026-11-17 (read-only). On main: one copy of the small helpers; one locked way to change a session's file. Phone: its checks are green again; the release recipe builds 灵境's files first. The rest of §5.3 is open |
| The phone and the Mac in step (§5.4) | The phone asks through declared tools: done 2026-11-17 for all four apps (music, 灵境's book, health, money) — the phone sends the Mac no shell text. The contract number: on main with §5.2. Open: one wording for "the Mac is not there", the session choice on the Mac |
| Asking less (§5.5) | The Bash gate reads a program's own file and a copy's source as read: on main 2026-11-17. A caller's preset of folders: on main. A coding session runs its work in its folder without asking, and a command that leaves the machine always asks, once: on main 2026-11-17, seen on a live delegated run. The list of build tools is gone. On main 2026-11-18: the gate says why a command asks or is blocked (nine reasons, worded in one place), and a coding session judges inline interpreter code like a script in its folder. In this worktree: literal line inspection needs only a read grant, including after `cd`; delegated briefs are not filed as the person's words |
| When Yinyue raises a problem (§5.6) | On main 2026-11-17, Mac, phone and skills: said once (again only when worse or returned), never dropped; a problem is a record that carries its offers or the reason there are none; a tap runs a declared tool as the person's own button, behind its confirm when destructive; the answer is kept on the problem's record; the phone measures what Linggen itself holds. Open: seen with buttons on a real phone; a hand-off offer; health, money and mission faults still raised the old way |
| 2.5 | 2.5.0 published 2026-11-19 (one MCP door), 2.5.1 on 2026-11-20 (a coding session that asks less and says what it cost). |
| 2.6 | The batch built after 2.5.1 as "2.5.2" ships as **2.6.0** (renumbered by Hanli 2026-11-21: 「改这么多, 可以按2.6发了吧」): the Mac app shows the web UI, rule files import, one decision point each for who started a turn, what a read may reach and what a person may send, the Claude Agent SDK session folded into Linggen sessions; `CHANGELOG.md` lists it. What earlier notes call 2.6 (the tool redesign, then a live talk with Yinyue) is now 2.7; what they call 2.7 (sharing within a household) is now 2.8. |
| 2.6.1 | Published 2026-11-22: the moon in the page shows the real phase, a private app called by name answers in its own session, the main page follows a message carried to another session. |
| 2.7 | Decided 2026-11-19: the tool redesign first (one catalogue for built-in tools, apps' tools and the MCP door; apps' tools found through a short index, not all sent each turn; an outside agent can use an app), then voice as a live talk — speech in and out as with a person, the camera on, a language teacher as the first use. State 2026-11-21: the live talk is not designed in detail or started. Built ahead of it on a branch, with fakes and never run on a real Mac: hold Yinyue and ask aloud (hearing on the Mac, `POST /api/stt`; her answer aloud; a spoken answer to a waiting question relayed only when it is exactly one option) — one way to speak with her, not the live talk. Also built on a branch and not released: the refactor step (store below the engine, one model rule, one turn entrance) and the tool redesign's steps one to three (one judge with a destructive class; the index and one tool for an app's actions; the door's thirteenth tool). |
| 2.8 Sharing | Built on its branch with fakes, 2026-11-21; not seen live with a second person's phone. Per person: level, apps, this Mac's models, a daily token limit (`server::people`, Settings → People), one ceiling for every turn that is not the owner's, and a phone is the account the relay vouches for. Rooms are untouched. Left: the phone's words and vouching, the relay's half on linggen.dev, the live trial. |

## 2. Milestones

| Release | Holds | Done when |
|---|---|---|
| **2.3** (published 2026-11-16) | Structure: architecture review, build speed, the merge, memory as a library the engine calls, the engine split into modules (first cut), patch and minor library upgrades, app slimming | Gate green on first install and on 2.2.1 → 2.3 (the takeover of an existing memory store); one process serving memory to Claude Code and Codex seen live on his Mac; sizes re-measured |
| **2.3.1** (published 2026-11-17) | The two major library groups (vector database; embedding stack), each verified on its own | Gate green on first install, 2.3.0 → 2.3.1 in place and back, and 2.2.1 → 2.3.1; published with one known cosmetic fault in 灵境 that 2.3.0 has too (a long name touching its status line in a crowded field row) |
| **2.4** | A release of its own (decided 2026-11-18, replacing the 2026-11-16 "ships as 3.0"): one Ling and one Yinyue (§5), with the day's session on by default; kinds of session, with a coding session any model can run (§5.1); a paired device's standing by account (§5.2); the refactor the second review asks for (§5.3); the phone slimmed and on the same version as the Mac. Added 2026-11-18: Yinyue speaks aloud only what the person cannot see; rooms (public and private) switched off by default, the code kept; memory warms its embedder at start and says so meanwhile. The Mac ships first; the phone's 2.4 follows once Apple has cleared 2.3 | Hanli has tested it and says so; gate green; the scenario tests pass; his three scenarios seen live on his Mac and phone |
| **2.5** | One MCP door (decided 2026-11-18): memory's tools join Linggen's, the X tools leave it — twelve tools at `/mcp`, the old memory door answers "update the plugin"; a model's readiness and why a run cannot go on are facts a caller reads; an unfinished run keeps its work; installed built-in missions follow the binary. **Built 2026-11-18 on branch `next/2.5`**, level with main, all suites green; tried on scratch engines and then on his Mac's engine (a real Claude Code session and a real Codex session on the door, a GPT model through the engine's own client, a refreshed dream mission with its schedule kept). Not on main: main is where the plugin installs from, so 2.5 reaches main only when its engine is published — after 2.4 is cut. After the door: check whether the Claude SDK kind of session is still needed — if a Linggen coding session does the work as well, it goes | On main and published; the plugin updated in both hosts; a night's dream on the folded tools seen clean |
| **2.6** | Renumbered 2026-11-21: the batch after 2.5.1 — the Mac app shows the web UI itself (one UI), rule files import, permissions decided in fewer places, the Claude Agent SDK session folded into Linggen sessions, a skill update keeps the person's files | Gate green on first install and on 2.5.1 → 2.6.0 |
| **2.7** | (Was 2.6.) The tool redesign first (decided 2026-11-19; the design agreed 2026-11-21): one catalogue for built-in tools, apps' tools and the MCP door; apps' tools found through a short index in the prompt and one `App` tool; an outside agent can use an app (the door gains a thirteenth tool); one place judges an app's action. Then voice as a live talk with Yinyue — speech in and out as with a person, the camera on, a language teacher as the first use — discussed after the tools are done. Built ahead of both, never run on a real Mac: hold her and ask aloud | — |
| **2.8** | (Was 2.7.) Sharing within a household, set per person (decided 2026-11-18; the details still to be gone through with Hanli): a level, apps, this Mac's models, a daily limit. Built on a branch with fakes, not seen live. Rooms, public and private, stay switched off with their code kept (decided 2026-11-18, for 灵境's online play later) — nothing of rooms is removed | — |
| **3.0** | A long-term goal, not a date: perhaps after 2.9. Decided when it comes | — |

## 3. Rules for the work

1. **Few general tools; the agent judges.** Hanli: 「我只是举了3个例子, 这样的
   例子有很多, 不能都通过工具或者代码来做. 工具要底层通用, 数量尽量少, 尽量让agent
   去做.」 A scenario is never a spec for a feature: no tool, coded condition
   or computed field per scenario.
2. **A move and a change of behaviour never share a commit.** Splitting moves
   code; it changes nothing a user or a test can see.
3. **Everything lands on main as it goes**, tests green; no long branch.
4. **Each release passes the gate** (`release-checklist.md`).
5. **The phone and the Mac ship together**, as far as the case allows (Hanli,
   2026-11-18: 「每次发版本, 手机, mac要一起发, 尽量, 根据情况定」). Same version
   number; the iPhone build goes to App Review first and the Mac is published
   when it is approved. A release that touches only one side may go alone —
   say so when planning it.

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
| Mac app installed | 359 MB (+ 109 MB ling-mem apart) | ≈ 290 MB (estimate) |
| Mac download (dmg) | 245 MB | — |
| ↳ 灵境 in the app | 209 MB (art ≈ 146, code 43, tools + tests 15) | ≈ 105 MB (art 61 + code 43; estimate from the measured packs) |
| ↳ `ling` | 132 MB | ≈ 170 MB merged (220 measured, less the 51 MB her clips no longer take) |
| iPhone upload (ipa) | 126 MB | ≈ 100 MB careful · ≈ 85 MB lean (estimates; the earlier 50–60 was wrong) |
| iPhone installed | 169 MB (灵境 110.8) | ≈ 144 MB careful · ≈ 124 MB lean (estimates) |

- **In the package:** the first chapter's art and the art every chapter
  shares. Sorted by a tool from the game's own data (`skills`
  `lingjing/tools/art-packs.mjs`, measured 2026-11-15, 137.5 MB of art in
  1,054 files): **package 60.9 MB** · later chapters 38.3 MB in ten packs ·
  **38.2 MB the game never loads** (untouched scans, paint sources kept beside
  the art, art of boards since cut) — these leave the package and are fetched
  by no one. The 60.9 MB is the careful count (every shared item, creature
  and card; every place an errand can reach); the play bot on an install
  holding only that list, with fetching blocked, shows what can still go.
- **The rest downloads:** Mac — all of it, in the background, right after
  first launch. Phone on Wi-Fi — the same. Phone on cellular — only the chapter
  that comes next; the rest waits for Wi-Fi.
- **The phone is another shape** (measured 2026-11-15, 110.8 MB of 灵境 in
  the app): its boards ride in 36 uncompressed packs that mix code and art —
  program code 19.6 MB (11.2 of it the hand-tracking wasm for 结印), the
  hand-tracking model 7.8, pictures 48.1 in packs + 19.9 of place paintings,
  audio 5.1, text 6.9. Only data may be fetched after install, never code.
  - No fetching needed: paint sources 1.75 MB, the `.ogg` copies the phone
    cannot decode 2.3 MB, JSON held twice 2.2 MB; jigsaw paintings stored at
    Mac size 4.4 MB (re-encoding them wants Hanli's eye).
  - Fetched by chapter: later chapters' place paintings 11.6 MB and their art
    inside packs 6.9 MB — the pack tool must first split art from code.
  - Careful ≈ 86 MB of 灵境 in the app; lean ≈ 65 MB (also fetches the
    hand-tracking model and art a first-chapter player does not reach).
- **Never packaged:** 灵境's `tools` and `tests` (15 MB, measured).
- **Yinyue stays in the package** (done 2026-11-15, engine main). Her 33
  clips are baked once, at build time, into a compact form: 52.4 MB → 1.1 MB
  (measured; largest error 0.19° per joint, 0.3 mm at the hips; rendered
  frames compared before and after). The web UI went from 78 MB to 27 MB
  (measured). No raw clip is in the program any more; the raw files and the
  baked ones are in no repository, so a build without them must say
  `LINGGEN_NO_CLIPS=1` (the Linux builds do). Her model (16.5 MB: textures 8,
  expression shapes 5.8) is left as it is.
- **Sources.** Our own files on linggen.dev's Cloudflare storage, each checked
  by content hash. Models: Hugging Face first, then the China mirror, then a
  copy on our storage for every model whose licence allows it; all pinned by
  hash. No China mirror for art yet: when Cloudflare cannot be reached the
  game plays on with pictures missing and retries.

## 5. 2.4 — one Ling, one Yinyue

Hanli: Ling is one person and Yinyue is one person, wherever they appear —
no "Yinyue in DJ" who differs from "Yinyue in 灵境". The unity comes from one
prompt and one injected world, not from moving transcripts around.

**Sessions.** Linggen makes the daily session, shared by all apps. Sessions
the user makes are unlimited and stay apart; they are given the same world.

**What code provides — and nothing about any scenario:**

| | What | Today |
|---|---|---|
| Now | World state: one line per app (declared by the skill), the machine, the body; in every session | Built (2.4 branch): app lines a skill declares (`world-line:`), the Linggen page in front, the same block in every session a person is in. The body appears once health declares its line |
| Past | One tool to read the activity log, filtered by time and app, returning totals | Built (2.4 branch): filters, totals with summed durations, stretches of use, verbs checked at the door, kept 400 days |
| Memory | Search and write | Built |
| Notes | A small note of her own, in front of her next time | Built (2.4 branch): `note_write` / `note_done`, per agent (Yinyue and Ling list them), beside every turn of the one who wrote them; only hers can carry a look-again time |
| Alarm | "Look again at / after" that she sets herself | Built (2.4 branch): a note's look-again wakes her once, a quiet turn |
| Voice and hands | Her words reach the user wherever they are; app actions the skills declare | Built |
| Linggen's daily session | App chats, her moments and phone hand-offs in the daily session (from 04:00); Ling and Yinyue seated; skill, tools, folder and grants per turn by the page; a private app in a session of its own | Built; `agent.daily_session` is on by default since 2026-11-18: the day's session, one rule for every way into it, a private app kept apart with the exit gate on its turns, a day per person, her moments in the app's place, the folded list. The phone's own change is not built |

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
are never told to her). Built (2.4 branch): the ceiling and quiet hours, quiet
hours letting through an unasked moment from the app in use, and a mission
fault waiting through them but outside the ceiling
(`yinyue-companion-spec.md` § Guardrails); a glance or woken note offers
only tools that do not act.

**Scenario tests** are how it gets smarter without more code: a world state
and a memory in, "should she speak, and naming what" out; rerun on every model
or persona change. The first three are Hanli's, kept as test fixtures: long work, long play and
a tired body, while he is in a game; a machine reading and a game state worth
acting on, while he is in a coding session; something he told her he cares
about, set against how the day has gone.

### 5.1 Kinds of session (agreed 2026-11-17)

**The need.** People use Linggen on different occasions — a chat, an app's
page, a game, writing code — and each wants a session set up for it. Adding a
new occasion later must not mean another special case.

**The way.** One description of a session (its *spec*): who is seated, which
tools, which folder, what memory, how permission is asked, how its context is
kept, where it is listed. A small table of presets names the kinds that exist
today (a chat the user starts, an app's session, Linggen's daily session,
Yinyue's own, a hand-off, a mission, a delegated run, a one-off). One place
applies a spec to a session; nothing else sets those things.

**Coding sessions.** A session's *work* can be `code`: it adds one section to
the prompt — how to work on code (look first, see a fault fail before fixing
it, change the least, never loosen a test, check, report what was and was not
verified) — with a few lines per model family. Nothing else changes: the
tools, plans, background tasks and permissions a session has already are
enough. Any model the user has a key for can run one. A coding session is one
the user (or a plugin: Claude Code, Codex) starts; it stays apart from
Linggen's daily session.

**Not doing.** No SDK per provider. No per-app switch for the user to set —
the page a turn comes from already chooses its place (§5).

**Proof.** `evals/code`: the same tasks run three ways — a model with the
`code` section, the same model without, and a Claude Code session.

### 5.2 A paired device's standing (decided 2026-11-17)

Hanli: 「配对设备, 看账号. 和mac上的账号一致, 是owner, 权限按owner处理. 非owner,
要可以mac上设置非owner权限. 应该是read权限.」

A paired device signed in to the Mac's own account is the owner's and may do
what the owner may. Any other paired device gets a level set on the Mac;
the default is read. A device at read keeps its own records (its chats, its
saves).

### 5.3 Review and refactor before 2.4

Hanli, 2026-11-17: 「3.0前还需要做一遍架构评审, 重构一下」 (said while this work
was numbered 3.0; it is 2.4 now). The second review looked for decisions made
in more than one place. Before 2.4:

- One copy of each small helper that had several.
- One writer for a session file's changes.
- One rule for which model a turn runs on.
- A turn's settings set through the session spec's one writer (after §5.1).
- One type for "who this person is", one ladder of permission levels (with §5.2).
- The store layer no longer reaches up into the engine.
- Old-format readers for versions before 2.0 removed (no real users yet).

Left for after 2.4: the further crate cuts, the event forwarder's rewrite,
typed turn errors, one way to start background work, skills declaring their
own setup steps.

### 5.5 Asking less (decided 2026-11-17)

Hanli: 「创建session的时候可以预设权限, 这样linggen就不用每次问了」. A delegated run
asked its caller again and again for the same harmless thing.

- **The gate first.** Running a program reads its file; a copy reads its
  source. Those are judged at read. A move and a hard link keep the full
  check on the source.
- **A coding session runs any program in its own folder** without asking
  (replacing, the same day, an owner's list of build tools: a list of
  programs stopped the compiler and let publishing through). What still
  asks, whatever the grants: a path outside the session's, the deny floor,
  and outward commands — publishing, remote operation, a push that names
  main/master, forces or deletes. A plain `git push` is not asked.
- **A caller's preset** of folders, when the session is made, within the
  caller's own level. Anything beyond is still asked, one at a time.
- One part of the session spec, filled the same way by a skill, a mission
  and a delegated run.

### 5.6 When Yinyue raises a problem (decided 2026-11-17)

Hanli: 「yinyue提示问题是对的, 最好能带一个解决方案, 让用户点一下, 由yinyue来解决问题,
不能只提出问题」 and 「不能看见问题不提」.

- A problem she raises carries a fix: one tap, and she does it — or, where
  she cannot, she says why and where the person can.
- She never stays silent about a problem she sees.
- Said once; again only when it is worse or has come back.
- No action without the tap. Every sentence is still hers; code hands her
  facts and the offer.

### 5.4 The phone and the Mac in step (decided 2026-11-17)

Hanli: 「engine升级时, 手机要同步升级吗?」 Today nothing answers that: the two
share no version, the phone learns the Mac changed only when a feature stops
working, and the App Store's review keeps them apart for days.

- **One number each side.** A contract number in one file on the Mac and one
  on the phone, kept equal by a test. The phone says its number when it
  connects; the Mac says its own. A side that is too old is told so once, in
  plain words, from one place — not by each feature failing its own way. No
  old behaviour is kept for an old phone (no real users yet).
- **The phone asks through declared tools.** The twelve places where the
  phone sends the Mac a shell command (the money, health and music apps'
  sync, 灵境's book) become tools the apps declare, one app at a time. This
  comes before §5.2: a phone at *read* cannot be allowed a shell.
- **One wording for "the Mac is not there"**, and the Mac — not a copy on the
  phone — decides which session a chat joins.
- **Release order.** The iPhone build goes to review first; the Mac release
  is published once it is approved; then the phone's release is tapped.
- **Smaller.** The release recipe builds 灵境's files from the skills first
  (a stale folder ships every picture). Next: the hand-tracking model fetched
  after install; code and art split in the board packs (after 2.4).

## 6. Before a step can ship

Checked on the web 2026-11-15 by a research agent. Several of the licensors'
pages refused automated reading, so each line says how firm it is; none of it
is legal advice.

| Item | Finding | How firm | So |
|---|---|---|---|
| Mixamo clips (26 `.fbx`) | Commercial use in a product is allowed; distributing the raw files to anyone outside the team is not | Adobe's FAQ read second-hand (the page refused the fetch) | Never host the `.fbx` files. Bake the clips into a compact form inside the program |
| Mixamo clips, converted | No Adobe text covers converted animation files on a public address | Unknown | Not hosted either, unless Adobe says so in writing |
| pixiv's 7 sample motions (`VRMA_01`–`07`) | Commercial use allowed **with the credit** "Character animation credits to pixiv Inc.'s VRoid Project"; distributing them or altered versions "in a way that can be rigged or extracted" is prohibited | Read on pixiv's own BOOTH page | Never hosted. Wherever the clips ship the credit is shown; keeping them or dropping them is Hanli's call (§8) |
| The VRoid model (`yinyue.vrm`) | Models made in VRoid Studio from its own presets may be used and distributed commercially | pixiv's guidelines through search excerpts, plus the Steam page read directly | May be hosted or packaged; set the usage fields in the file |
| Picture model FLUX.2 [klein] 4B | Apache-2.0: may be mirrored and used commercially; the 9B and [dev] variants are non-commercial | Model card and the maker's own announcement, read directly | May be mirrored with its licence text; still move off the personal account's conversion |
| Cloudflare storage | $0.015 per GB-month, no egress fee, 10 GB free | Pricing page, updated 2026-10-30 | ≈ $0.03–0.20 a month for 12 GB and 2,000 downloads |

Consequence for Yinyue: her animations stay in the program, baked small
(done); only her model would be free to move, and at 16.5 MB it stays too.

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
- Decided 2026-11-16, renumbered 2026-11-18: the phone goes to 2.4 with the Mac (an iPhone 2.3 for the published Mac 2.3.1 goes out first); the careful count; 结印's camera stays. iPhone 2.3.0 (build 45) was submitted on 2026-11-18 and awaits App Store review. It answers the 2.2.0 rejection.
- How far apart people who share one Mac are kept beyond the day's session.
- Whether a person's "today" starts at 04:00 everywhere (the daily session's does), not only in the daily session.
- How the world state reaches a session Linggen does not host (Claude Code or
  Codex in a terminal) — through the plugin's hook, or not at all.
- Whether the gate tests a first launch with no network.
- pixiv's seven sample motions: show the credit and keep them (baked), or drop them.
