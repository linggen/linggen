---
type: design
reader: Coding agent and Hanli
status: designed 2026-10-05, not built
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

Each test starts its own world and throws it away:

- **Engine** on a random port, `LINGGEN_HOME` in a temp dir, env cleared
  (`env -i` + an allow-list). Never `~/.linggen`, never 9527/9528.
- **ling-mem** as a scratch daemon with its own HOME and a seeded store —
  the pattern `linggen-memory/scripts/live-check.sh` already uses. Most tests
  use a stub embedder or a pre-built store; the real 1.2 GB model only in a
  few.
- **Fake model: llmposter** (CLI, test-only; AGPL is fine for a tool we never
  ship). It speaks OpenAI Chat + Responses, Anthropic and Gemini with SSE,
  tool calls and scripted multi-turn sequences, so a test says "turn 1 calls
  Look, turn 2 answers X" and gets exactly that. A small wiremock stub covers
  Ollama.
- **Snapshots: insta** — system-prompt exports per member, each member's
  built thread, history API output. A change shows as a diff to review.
- **Web UI: Playwright** against the test engine; Chromium with
  `--use-fake-ui-for-media-stream --use-fake-device-for-media-stream` so the
  WebRTC transport is the real one.

**Fixtures** live in the repo (`tests/fixtures/home/`), copied per test:
config pointing at the fake model; small test skills (never a real app like
Lingjing — it changes and has its own owner); prewritten sessions (shared,
compacted, interrupted run); one mission; invented memory rows (a person
called Alex, a few made-up projects). Real data never enters this layer.

First cases to port: the shared-session scenarios now in
`scripts/live-check.sh` (a, b, d, e, f), the interrupted-run display,
presence per surface, session `withheld_tools`, memory in the prompt.

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
Mac: `just release-gate <versions>`.

**First install** (fresh VM, no `~/.linggen`):
1. The real public paths: `install.sh`, `install-app.sh`,
   `install-shared-memory.sh`, the Claude Code and Codex plugin installs —
   pointed at the draft assets.
2. Also one browser-style download with `com.apple.quarantine` set
   (`install-app.sh` strips it, so curl alone hides Gatekeeper).
3. Pass when: 9527 health; the UI and first-run onboarding load
   (Playwright over an SSH tunnel); ling-mem `session_start` answers; one
   chat turn completes on the fake model.
4. Fail when: any TCC prompt or Gatekeeper dialog appears on the first-run
   path (watched via `log stream` / a screenshot), or the pair dialog fires.
5. One real Linggen Cloud turn as information, not a blocker.

**Upgrade** (old-user VM `linggen-prev`: previous release + fixtures home with
sessions, config, saves, a memory store in the old schema, both plugins):
1. `ling update`, the in-app updater, `ling-mem upgrade --yes`, plugin
   updates.
2. Pass when: the store migrates; built-in missions refresh; old sessions,
   config and saves load; plugin caches carry the new hooks.
3. Rollback: a release with a wrong sha256 is refused and the old binary
   still runs.
4. A green gate saves its VM as the next `linggen-prev`, so the old-user
   baseline rolls forward each release.

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
5. Tart release gate: first install, then upgrade; `just release-gate`.
6. Wire the gate into `release-checklist.md` between draft and publish.
