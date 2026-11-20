---
type: design
reader: Coding agent and Hanli
status: built 2026-10-05; hardened after review 2026-10-06
guide: |
  How Linggen is tested before and after a release — layers, tools, data.
  Brief; the scripts are the truth once built, this page is the shape.
---

# Test design

Goal: every release ships only after one automated run says it works — the
code is right, a new user's first install starts and runs, and an existing
user upgrades cleanly.

Test harnesses and release-gate runs opt out of telemetry and are never counted as users.
Gate VMs keep the opt-out file so engines launched by the app are covered too.

## Layers

| When | What | Where | Data |
|---|---|---|---|
| Every change | Unit tests | host | made up in the test |
| Before the draft | System tests | host, hermetic | fixtures home + fake model |
| Before the draft | Plugin evals | Claude Code | throwaway workspace |
| Draft → publish | Release gate: first install + upgrade | clean macOS VM (Tart) | fixtures home |
| After deploy | Smoke on real data | Hanli's Mac, 9527 | real, read-only |

Fit to `release-checklist.md`: each repo's `release.sh --draft` uploads the
assets; the gate tests those draft assets; only a green gate runs
`gh release edit --draft=false --latest`. Nothing reaches users (the in-app
updater, `install.sh`) before the gate passes.

## 1. Unit tests

Each stack's own framework: `cargo test` (engine, ling-mem), `node --test`
(UI, shared page helpers, skills). `./scripts/check.sh` runs them all and is
what CI runs. Use `cargo nextest` so tests that touch processes or ports run
isolated.

### Memory's contract suites

Before memory's handlers move from "a service the engine hosts" to "a
library the engine calls", its current behaviour is pinned by offline tests
(no model, no network beyond loopback, ~3 s):

- **Test embedder** (`linggen-memory`'s `test-support` feature, never in a
  normal build — `tests/contract/shipping.rs` checks the resolution):
  feature hashing of the text's words into the 1024 dimensions, so equal
  texts are equal vectors and similarity follows shared words.
  `hosted::open_quiet_fake` opens a store on it.
- **`crates/linggen-memory/tests/contract/`** — the real hosted router over
  loopback HTTP on a scratch store; each test asserts the status and the
  whole envelope (ids and stamps scrubbed). `verbs` every row verb's happy
  path; `guards` the refusals and merge rules, word for word where a model
  reads them; `person` per-person rows; `upkeep` days, sweep, chains,
  issues, session start, stats, config, sync, the schema step, health;
  `layers` replica write-forwarding into `.outbox.json`, primary-only
  refusals, the activity clock; `mcp` the `/mcp` endpoint; `replica` two
  stores, B made A's replica through the real routes — forward, pull, and
  an outbox replayed after A was unreachable. A behaviour that looks wrong
  is pinned as it is today and changed only on purpose.
- **`src/server/api/memory/door_tests.rs`** — the engine's door
  `/api/memory/<verb>` on such a store, as the owner, the owner's phone and
  another person's phone: which verbs each runs, the person stamp, statuses
  passed through, 502 `UNREACHABLE`.

## 2. System tests — hermetic, on fixtures

`tests/system` (`./scripts/check.sh system`, i.e. `cargo nextest run --test
system`; part of `all`; ~7 s of tests; in CI as the `system` job on the
latest released ling-mem, without the embedding model). Each test starts its
own world and throws it away:

- **Engine**: the built `ling --web` as its own process on a random port,
  `HOME`/`LINGGEN_HOME`/`TMPDIR`/`XDG_*` in the test's root, env cleared
  (`env -i` + an allow-list), outbound HTTP sent to a closed proxy. A guard
  panics before anything starts if a world would reach `~/.linggen` or
  9527/9528 (env, every file of the rendered home; 9528 is the old
  memory daemon's, still real on a machine before 2.3); after a passing test the engine log and every model request
  must never name 9527/9528. Each child leads its own process group: a stop is
  TERM to the group, KILL after 5 s, also when the test process dies. A server
  counts as up only when it answers health and is alone on its port (the
  listener is the child's); a taken port moves to a fresh one. Roots live in `/var/tmp/linggen-system-tests`: never `/tmp` or
  `$TMPDIR` (always-writable scratch to the engine, so a write would never
  ask), never the OS temp dir (never a project), never inside a git repo. A
  failed test keeps its root; roots older than 3 days are swept.
- **ling-mem** as a scratch daemon over the world's home, the same `serve`.
  It has no stub embedder: by default its model load is refused offline, so
  `session_start`, `list` and MCP work and an add or search fails fast. The
  few tests that need rows clone the model from the person's Hugging Face
  cache (copy-on-write, never a link — the real cache is never written) and
  seed invented rows (~2 s). None is `#[ignore]`d: they run when the model is
  present (`scripts/scratch-test.sh system` links the machine's cache into
  its scratch home for them, as the release gate does); without it each
  names itself (`support::skipped`) and the script lists them as SKIPPED at
  the end; `LINGGEN_SYSTEM_REQUIRE_EMBED=1` fails the prompt test without it.
  The suite prints the ling-mem path and version it runs.
- **Fake model: llmposter** as a dev-dependency running inside the test
  process (AGPL is fine for a tool we never ship; it never reaches the
  shipped binary). In-process rather than its CLI: each test gets its own
  server, scenario state and the captured requests — what each member was
  actually sent. A member is told apart by its system prompt, so a test says
  "Ling: call Look, then say X; Yinyue: say Y" and gets exactly that; the
  Responses API path runs on fake ChatGPT tokens. An Ollama stub is not built
  yet. ChatGPT's picture generation (an `image_generation_call` llmposter
  cannot send) is a loopback stand-in in the test (`tests/system/drawing.rs`);
  `Setup::image_api` points the engine at it through
  `LINGGEN_IMAGE_API_URL`.
- **The event stream**: each world's engine runs with
  `LINGGEN_EVENT_LOG=<root>/events.jsonl` — every UI event it sends, one JSON
  line each (off unless set) — so a test reads what a surface would see
  (`World::events`) beside the rows on disk (`runs.rs`: every run event and
  row names its run).
- **Snapshots: insta** — system-prompt exports per member, each member's
  built thread, history API output. A change shows as a diff to review.
- **Web UI: Playwright** against the test engine; Chromium with
  `--use-fake-ui-for-media-stream --use-fake-device-for-media-stream` so the
  WebRTC transport is the real one.
  Built: `tests/e2e` (`./scripts/check.sh e2e`, ~1 min, not in `all`); each
  test starts its world through `tests/world`, the same world-builder run as a
  process, on the debug `ling` (it serves `ui/dist` from disk). Every test
  checks the page's plain-HTTP requests against an allow-list (the page,
  `/assets`, `ui/public` files, `/api/rtc/*` signaling); the UI's remaining
  HTTP senders (presence, TTS, `/api/*` before the transport connects) are
  named in `tests/e2e/support/world.ts` until they move to the data channel.
  Counts wait for a definite turn end (the chat box's `data-busy="false"`),
  never a sleep; selectors are `data-testid`/roles. A bug found but not fixed
  stays a `test.fail` with a dated note. The first-run world has no
  config at all (`Setup::no_config`, scenario `noConfig`): the engine on its
  defaults, its port from `--port`, its memory the store it hosts there.

**Fixtures** live in the repo (`tests/fixtures/home/`), copied and rendered
(`{{PORT}}`-style placeholders) per test: config pointing at the fake model;
small test skills (`dice`, `quiet` — never a real app like Lingjing, it
changes and has its own owner); prewritten sessions (shared, compacted,
interrupted run); one mission; invented memory rows (`tests/fixtures/memory/`:
a person called Alex, made-up projects). Real data never enters this layer.

Ported first: the shared-session scenarios of `scripts/live-check.sh`
(a, b, d, e, f), the interrupted-run display, presence per surface, session
`withheld_tools`, memory in the prompt, the export's tools per member. The
live script stays for the real-data smoke (§ 5).

## 3. Plugin evals — `claude plugin eval`

Claude Code's own harness tests the linggen plugin (memory hooks, the
`linggen` skill, `/linggen:status`) in a throwaway workspace with only the
plugin loaded, graded by `tool_used` / `tool_order` / `regex`, gated with
`--threshold`. It calls a real model on the signed-in account (a subscription
works, no API key), so it runs before a release, not on every change:
`./scripts/plugin/plugin-eval.sh`. `scripts/plugin/plugin-check.sh` stays
for parity of the installed copies and the hook proxy runs.

Built (2026-10-05) differently from the plan in three ways:
- **The door's MCP server is mocked, not real.** A run passes only an
  allowlisted environment, so `.mcp.json` can't be pointed off 9527. The
  model sees mocks under `mocks/linggen/` (one per tool, answering each of
  its ops) and the door's tools list as `mocks/linggen/_tools.json`; each
  case carries the door's server text as its system prompt. The hooks go to
  a scratch engine with invented rows instead, via the run's
  `~/.linggen/client.json`. A unit lock (`src/server/mcp/repo_check.rs`)
  keeps the snapshot and the server text equal to the door's own. The suite
  lives in `plugins/linggen/evals-isolated/`, so a bare
  `claude plugin eval .` finds nothing.
- **No Bash in runs.** The eval sandbox refuses Bash on this Mac (a symlink in
  `~/.docker`), so cases use only Skill and the mocked tools.
- **Stamps are graded through the mock.** The `memory_add` mock echoes what
  it received after `stamp-cwd.sh`. The 89325e8 session-root fix can't be
  reproduced here (eval workspaces are temp dirs) and stays in
  `plugin-check.sh`.

## 4. Release gate — clean Mac, Tart

Tart clones a vanilla macOS image copy-on-write in about a second and is
driven over SSH. Apple's licence allows two macOS VMs per Mac — enough for
one fresh VM and one old-user VM. GitHub's macOS runners are not clean (brew,
node, python preinstalled) and cannot run Tart, so the gate runs on Hanli's
Mac: `just release-gate --draft "engine=… app=…"` (or `--local` for local
builds) — `scripts/gate/`. Memory is inside `ling` and checked through it;
the plugin bundle is the engine draft's asset (packed from the checkout for
`--local`). Image: `linggen-gate-base` when it exists —
`macos-tahoe-vanilla` switched by hand to "App Store & Known Developers"
(`release-checklist.md`) — else `macos-tahoe-vanilla` (~27 GB, pulled once);
`GATE_IMAGE` overrides. Before any VM boots the gate runs `check.sh all` and
`check.sh e2e` on the engine HEAD it records (red stops it;
`--skip-host-checks` is a GAP), and for `--draft` compares HEAD to the commit
the engine draft is cut from: a sha target that differs FAILs, a moved branch
WARNs. Then 灵境 is played on the released skills — Linggen.app's vendored commit and the phone pack's — by
the play bot (`skills/lingjing/tools/playbot`, `just playbot`; scratch install and saves, headless): hard checks
on every tap, a layout lint and pixel baselines at phone and Mac sizes, and a review pack (`checklist.md`,
`review.json`) a reviewer answers per screenshot. A hard finding FAILs; an unanswered review (or one for another
run id) is REVIEW and keeps the gate from green; the phone's `lingjing_*` tests run on assets built from the same
commit, a skip is a GAP; `--skip-playbot` is a GAP. `just playbot --selftest` must catch every seeded bug.
Every VM script ends with an exit line; one that dies early or exits
nonzero FAILs ("ran to the end"). The VM reaches the host's mirror and llmposter through SSH reverse
tunnels, so nothing on the host listens beyond loopback.

**First install** (fresh VM, no `~/.linggen`):
1. The real public paths: `install.sh`, `install-app.sh`,
   the retired memory installer (says so, exits 0), the Claude Code and
   Codex plugin installs — pointed at the draft assets. A clean Mac's git is
   the Command Line Tools stub, so the plugins go through
   `install-plugin.sh`: it must see no git and install the engine release's
   `linggen-plugin.tar.gz` as `linggen@linggen` into both hosts with no CLT
   dialog (FAIL if the dialog is up). The GitHub marketplace form is still tried and reported.
   Codex has no unattended installer without node; the host's CLI is copied in.
   Claude Code comes from the live claude.ai/install.sh: its version is INFO,
   its failure a GAP.
   After `install-app.sh` (bun removed first, so it is that script's own): bun
   is in `~/.linggen/bin`. Then the DMG state, made by hand: app quit, 9527
   down, bun removed, no bun/node where `run-js.sh` looks (else a GAP) — the
   app opened must bring bun back by itself and a 灵境 Look must answer ok
   (unsigned, the door is 401, so the Look runs as the declared command).
   The app's first launch heals its slimmed 灵境: the installed folder lacks
   every file its `assets.json` lists, and within 10 min `GET /api/assets/lingjing`
   holds them all with 0 failing (the VM reaches assets.linggen.dev; no route
   there is a GAP). After the upgrade the same; a baseline copy that keeps the
   files is a GAP for the fetch.
2. Also a browser-style download of each asset — the app, `ling` (and
   no `ling-mem`: it is not an asset) — with `com.apple.quarantine` set (curl and `install-app.sh`
   leave none, so they alone hide Gatekeeper). The vanilla image ships with
   Gatekeeper off; the gate turns it on first, but only as "App Store" —
   macOS 15+ cannot switch to "App Store & Known Developers" from a shell, so
   every Developer ID app is refused there. Pass: `spctl` names the source
   `Notarized Developer ID` (or accepts; a bare CLI, "not an app" to
   `-t exec`, is asked with `-t install`); the quarantined run (the app
   through `open`, the CLIs `--version`) decides, and is a GAP on the vanilla
   image (no `linggen-gate-base`). An asset that is not
   Developer ID signed FAILs a `--draft` run and WARNs a `--local` one; a
   `--draft` with no `app=` serves the published app, not this train's, so
   its verdict is a GAP.
3. Pass when: 9527 health; the UI loads (Chromium from `tests/e2e`'s
   Playwright over an SSH tunnel); memory answers through the engine —
   `/mem/api/health`, `/mcp` initialize + tools/list (twelve tools; `/mcp/memory` answers 410), `ling mem
   session-start` and `stats` — with `~/.local/bin/ling-mem` the stand-in
   (`install.sh` lays down no ling-mem, so there is no `ling-mem.prev`), no
   ling-mem process, nothing listening on 9528, `/mem/mcp` answering on 9527
   and the stand-in's `status` naming 9527; after the plugin installs the
   stand-in stays, no daemon runs and nothing listens on 9528; one chat turn completes on the fake model.
4. Fail when: a TCC prompt appears on the first-run path — `log stream` of
   tccd; a Linggen request with `preflight=no` that a person answered, timed
   out or never answered is a prompt.
5. Not yet checked: the pair dialog, the Local Network prompt (not tccd),
   one real Linggen Cloud turn.

**Upgrade** (old-user VM `linggen-prev`: previous release + fixtures home with
sessions, config, saves, a memory store in the old schema, the Claude Code
plugin `linggen@linggen-memory`; Codex is handed in at the plugin step):
1. `ling update`, `install-app.sh`, plugin updates. A local build with the baseline's own version
   number updates from the relabelled manifest. Releases up to engine 1.8.2 / ling-mem 1.8.2 / app 0.3.3 predate
   `LINGGEN_RELEASE_BASE`, so from them the gate re-runs `install.sh` (a GAP
   row) over the running install — its first run caught `install.sh` copying
   over a live `ling` (killed on launch; fixed: fresh inode + rename). After
   the swap 9527 must be a new process. An old `ling update` from before
   the engine restart is a GAP; the release's own `ling update` and
   `--rollback` are then checked over a running engine (PASS/FAIL).
   The in-app updater asks through a native dialog and is not driven.
2. Takeover (2.3.0 on, `vm/takeover.sh`; the baseline is 2.2.1 + ling-mem
   1.13.0 with its daemon running): rows written through the old daemon —
   three long-term, one staged, one core. After the upgrade, each a check:
   the old daemon is gone; nothing listens on 9528;
   one `memory/backups/takeover-<stamp>/` and `takeover-<version>.done`; the
   store lock names the engine's pid; every earlier row is found by id
   through the engine, at its tier, and the core row is in `ling mem
   session-start`; `~/.local/bin/ling-mem` is the stand-in and
   `ling-mem.prev` the old binary; MCP answers at `9527/mem/mcp` and
   `9527/mcp` (and `9527/mcp/memory` answers 410); a write through the stand-in lands. Rollback: the
   previous engine and `ling-mem.prev` back and started find every row,
   including those written after the upgrade; the release again stops the
   old daemon again and the marker keeps a second backup from being made.
   A baseline with no old daemon makes these a GAP.
3. Pass when: the store opens with every row (and `apply-schema --yes` keeps
   them); old sessions, config and a chat turn on the old config work;
   `install-plugin.sh` retires `linggen@linggen-memory` (the 2.2.x plugin
   `linggen-prev` has, as the builder's Macs do) and leaves `linggen@linggen`
   enabled in Claude Code and Codex on the new bundle's hooks, the old id in
   neither, nothing on 9528. Built-in missions are install-once
   (`cli/init.rs`), so an old copy is reported, not failed.
4. Rollback: a release with a wrong sha256 is refused and the old binary
   still runs; `ling update --rollback` swaps to `.prev` — the pre-upgrade version (the gate keeps the old binary
   aside and puts it there when the update kept none or kept the release's own
   bytes) — and back. With nothing older to return to (the baseline already
   runs the release) the rollback and restart checks are GAPs, not passes.
   `ling update --rollback` from 2.3 to an engine that does not host memory
   puts `ling-mem.prev` back over the stand-in before that engine starts (it
   starts memory through `ling-mem`): the old daemon answers on 9528 with every
   row, including those written after the upgrade (to one that hosts memory:
   9527/mem does), and rolling forward takes
   the store over again (PASS/FAIL). The automatic roll-back of an unhealthy
   restart does the same.
5. A green `--draft` gate saves its VM as the next `linggen-prev`, so the
   old-user baseline rolls forward each release; `--local` keeps it unless
   `--save-prev`. `linggen-prev` is built once from the published releases.
   That rolling baseline tests N-1 → N only; `--prev <vm>` (or
   `GATE_PREV_VMS`) also upgrades frozen older snapshots (`tart clone
   linggen-prev linggen-prev-1.8`) in the same run, never saved over.

Needs from the engine first — built 2026-10-05: `LINGGEN_RELEASE_BASE`
points every installer and updater at the gate's mirror of the drafts
(`cli.md` § Release override); `ling update` keeps
`.prev`, restore it when the new binary doesn't start, and take
`--rollback`; the app's updater keeps and restores the previous bundle.

### The one MCP door (2.5)

Before a release that touched `/mcp` or the plugin, on a scratch store and a
scratch home (never the real store), with fake embeddings and a fake model:

1. **The list.** `tools/list` on this machine is exactly twelve tools, in
   order: `memory_search`, `memory_add`, `memory_update`, `memory_delete`,
   `memory_upkeep`, `agent_run`, `agent_status`, `agent_permit`, `agent_end`,
   `browser_open`, `browser_read`, `browser_act`. The test builds the expected
   list from the one catalogue, not from a second hand-written list. An
   approved computer's list is the five memory tools; a phone and an unpaired
   off-machine caller are refused; a direct `tools/call` of a tool or op
   outside the caller's view is refused.
2. **The server text** is at or under 2,048 characters in each view
   (`instructions.chars().count()`), and still says the key things: the memory
   rules, the watcher rule, the permission decisions, the usage-limit sentence
   and the browser sentence in the local view.
3. **Strict schemas.** Per op: the minimum arguments, a bad value, and a field
   of another op are each refused with the form that works.
4. **The agent flow on a fake run.** A permission request raised before the
   watcher connects; a watch that closes with no `finished` then one
   `agent_status op=session` and a new watch; a continuation with `session_id`;
   a request left unanswered is a deny at the timeout; `agent_end op=stop`
   releases a waiting run; an outward request takes `allow_once` or `deny`
   only; the deny floor never produces a request that can be approved;
   `agent_end op=close` never removes a worktree that is not clean and on a
   remote branch.
5. **Models and the usage limit.** `agent_status op=models` shows `ready` and
   the reason for a model that is not; a run that stops on a usage limit
   carries `cannot_run` with its reset time in `finished` and in
   `agent_status op=session`.
6. **The browser.** `browser_read op=page`, then `browser_act`, then a read
   again after the page changes; the site confirmation in Chrome is answered
   and a decline is reported.
7. **The old plugin.** `/mcp/memory` answers HTTP 410 `UPGRADE_PLUGIN` with
   the update text and forwards nothing; a retired tool name and an old shape
   (`agent_status` with only `session_id`) run nothing and answer with the
   update message and the new form. A repo check keeps retired names, the old
   retired `/mcp/memory` address and the old plugin namespace out of the live plugin,
   commands, graders and docs, except under a history heading or an explicit
   upgrade fixture.
8. **Two hosts, two installs.** On Claude Code and on Codex, each (a) on a
   clean install and (b) on an install where the old plugin is still present:
   session start; a search; a save; a delegated run that raises a permission
   request, answered; a browser read. On (b) the old plugin's connection is
   refused with the update message; where the host shows only a generic
   disconnect, or the old per-turn hook hides the error, that row is a GAP,
   not a pass.

## 5. Smoke on real data

`./scripts/check.sh live` against Hanli's own 9527 after every deploy: real
config, models and stores, read-only; scratch sessions withhold memory tools
and are deleted after. It catches what fixtures can't: his real models, real
skills, real network. Model cases use real providers, so a few stay; the rest
run with `--no-model`. One guards Ling's answer length in a plain chat (case
g): three real questions must get a full answer (≥150 CJK chars or ≥80
words), and "hi" a short one (≤300 chars). `--url`/`--home` run it against a
scratch engine instead of 9527.

## Real-model checks

Agents are not deterministic, so real models never gate a change. A small
set runs before a release (plugin evals, the smoke run, optionally Promptfoo
with fixed rubrics) and reports; a regression there is read by a person.
Recorded real responses are only a source for new llmposter fixtures.

## Not covered by automation

Real TCC grants (Photos, Full Disk Access, Automation), osascript dialogs,
Metal TTS and embedding speed, phone ↔ Mac over a real network, live sites
(stockanalysis.com, SEC). Linux is out of scope while it has no users; the
`build-linux.yml` dry run in the checklist stays as is.

## Build order

1. Engine harness + fixtures + llmposter; port the live-check scenarios.
2. Playwright on the harness.
3. `claude plugin eval` suite for the linggen plugin.
4. Engine: release-URL override, rollback in `ling update`. (built)
5. Tart release gate: first install, then upgrade; `just release-gate`. (built)
6. Wire the gate into `release-checklist.md` between draft and publish. (done)
