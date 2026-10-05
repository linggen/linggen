---
type: design
reader: Coding agent and Hanli
status: designed 2026-10-05; § 2 engine harness built (tests/system)
guide: |
  How Linggen is tested before and after a release — layers, tools, data.
  Brief; the scripts are the truth once built, this page is the shape.
---

# Test design

Goal: every release ships only after one automated run says it works — the
code is right, a new user's first install starts and runs, and an existing
user upgrades cleanly.

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

## 2. System tests — hermetic, on fixtures

`tests/system` (`./scripts/check.sh system`, i.e. `cargo nextest run --test
system`; part of `all`, ~10 s; not in CI until CI fetches a ling-mem). Each
test starts its own world and throws it away:

- **Engine**: the built `ling --web` as its own process on a random port,
  `HOME`/`LINGGEN_HOME`/`TMPDIR`/`XDG_*` in the test's root, env cleared
  (`env -i` + an allow-list), outbound HTTP sent to a closed proxy. A guard
  panics before anything starts if a world would reach `~/.linggen` or
  9527/9528. Roots live in `/var/tmp/linggen-system-tests`: never `/tmp` or
  `$TMPDIR` (always-writable scratch to the engine, so a write would never
  ask), never the OS temp dir (never a project), never inside a git repo.
- **ling-mem** as a scratch daemon over the world's home, the same `serve`.
  It has no stub embedder: by default its model load is refused offline, so
  `session_start`, `list` and MCP work and an add or search fails fast. The
  one test that needs rows loads the real model through a link to the
  person's Hugging Face cache and seeds invented rows (~2 s warm, ~16 s
  cold; skipped when the model is absent).
- **Fake model: llmposter** as a dev-dependency running inside the test
  process (AGPL is fine for a tool we never ship; it never reaches the
  shipped binary). In-process rather than its CLI: each test gets its own
  server, scenario state and the captured requests — what each member was
  actually sent. A member is told apart by its system prompt, so a test says
  "Ling: call Look, then say X; Yinyue: say Y" and gets exactly that; the
  Responses API path runs on fake ChatGPT tokens. An Ollama stub is not built
  yet.
- **Snapshots: insta** — system-prompt exports per member, each member's
  built thread, history API output. A change shows as a diff to review.
- **Web UI: Playwright** against the test engine; Chromium with
  `--use-fake-ui-for-media-stream --use-fake-device-for-media-stream` so the
  WebRTC transport is the real one.
  Built: `tests/e2e` (`./scripts/check.sh e2e`, ~1 min, not in `all`); each
  test starts its world through `tests/world`, the same world-builder run as a
  process, on the debug `ling` (it serves `ui/dist` from disk).

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
`./scripts/check.sh eval` in linggen-memory. `scripts/plugin-check.sh` stays
for parity of the installed copies and the hook proxy runs.

Built (2026-10-05) differently from the plan in three ways:
- **ling-mem MCP is mocked, not real.** A run passes only an allowlisted
  environment, so `.mcp.json` can't be pointed off 9528. The hooks go to a
  scratch daemon with invented rows instead, via the run's
  `~/.linggen/client.json`. The suite lives in
  `plugins/linggen/evals-isolated/`, so a bare `claude plugin eval .` finds
  nothing.
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
Mac: `just release-gate --draft "engine=… mem=… app=…"` (or `--local` for
local builds) — `scripts/gate/`. Image: `macos-tahoe-vanilla` (~27 GB, pulled
once). The VM reaches the host's mirror and llmposter through SSH reverse
tunnels, so nothing on the host listens beyond loopback.

**First install** (fresh VM, no `~/.linggen`):
1. The real public paths: `install.sh`, `install-app.sh`,
   `install-shared-memory.sh` (retired: says so, exits 0), the Claude Code and
   Codex plugin installs — pointed at the draft assets. A clean Mac's git is
   the Command Line Tools stub, so the plugins go through
   `install-plugin.sh`: it must see no git and install linggen-memory's
   `linggen-plugin.tar.gz` into both hosts with no CLT dialog (FAIL if the
   dialog is up). The GitHub marketplace form is still tried and reported.
   Codex has no unattended installer without node; the host's CLI is copied in.
2. Also a browser-style download of each asset — the app, `ling`,
   `ling-mem` — with `com.apple.quarantine` set (curl and `install-app.sh`
   leave none, so they alone hide Gatekeeper). The vanilla image ships with
   Gatekeeper off; the gate turns it on first, but only as "App Store" —
   macOS 15+ cannot switch to "App Store & Known Developers" from a shell, so
   every Developer ID app is refused there. Pass: `spctl` names the source
   `Notarized Developer ID` (or accepts); the quarantined run (the app
   through `open`, the CLIs `--version`) is a GAP until `GATE_IMAGE` names an
   image switched by hand. An asset that is not Developer ID signed FAILs a
   `--draft` run and WARNs a `--local` one.
3. Pass when: 9527 health; the UI loads (Chromium from `tests/e2e`'s
   Playwright over an SSH tunnel); ling-mem `session_start` answers (ling-mem
   starts on first use, so the gate starts it); one chat turn completes on
   the fake model.
4. Fail when: a TCC prompt appears on the first-run path — `log stream` of
   tccd; a Linggen request with `preflight=no` that a person answered, timed
   out or never answered is a prompt.
5. Not yet checked: the pair dialog, the Local Network prompt (not tccd),
   one real Linggen Cloud turn.

**Upgrade** (old-user VM `linggen-prev`: previous release + fixtures home with
sessions, config, saves, a memory store in the old schema, both plugins):
1. `ling update`, `ling-mem upgrade --yes`, `install-app.sh`, plugin
   updates. Releases up to engine 1.8.2 / ling-mem 1.8.2 / app 0.3.3 predate
   `LINGGEN_RELEASE_BASE`, so from them the gate re-runs `install.sh` (a GAP
   row) over the running install — its first run caught `install.sh` copying
   over a live `ling` (killed on launch; fixed: fresh inode + rename). After
   the swap 9527 and 9528 must be new processes, 9528 on the new version
   (`install.sh` restarts the engine, `install-bin.sh` and `ling-mem upgrade`
   the daemon, `ling update` the engine). An old `ling update` from before
   the engine restart is a GAP; the release's own `ling update` and
   `--rollback` are then checked over a running engine (PASS/FAIL).
   The in-app updater asks through a native dialog and is not driven.
2. Pass when: the store opens with every row (and `apply-schema --yes` keeps
   them); old sessions, config and a chat turn on the old config work;
   `install-plugin.sh` moves both plugins to the new bundle's hooks. Built-in missions are install-once
   (`cli/init.rs`), so an old copy is reported, not failed.
3. Rollback: a release with a wrong sha256 is refused and the old binary
   still runs; `ling update --rollback` and `ling-mem upgrade --rollback`
   swap to `.prev` and back.
4. A green `--draft` gate saves its VM as the next `linggen-prev`, so the
   old-user baseline rolls forward each release; `--local` keeps it unless
   `--save-prev`. `linggen-prev` is built once from the published releases.

Needs from the engine first — built 2026-10-05: `LINGGEN_RELEASE_BASE`
points every installer and updater at the gate's mirror of the drafts
(`cli.md` § Release override); `ling update` and `ling-mem upgrade` keep
`.prev`, restore it when the new binary doesn't start, and take
`--rollback`; the app's updater keeps and restores the previous bundle.

## 5. Smoke on real data

`./scripts/check.sh live` against Hanli's own 9527 after every deploy: real
config, models and stores, read-only; scratch sessions withhold memory tools
and are deleted after. It catches what fixtures can't: his real models, real
skills, real network. Model cases use real providers, so a few stay; the rest
run with `--no-model`.

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
