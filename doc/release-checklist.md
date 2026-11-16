# Release checklist

One train, in order: engine → Mac app. Memory runs inside `ling`, and the
Claude Code / Codex plugin bundle is an asset of the engine release: there is
no ling-mem or plugin release of its own. Each step names the script that does it; the script is
the truth, this page is the order.

## Release gate — between each draft and its publish

The gate (`doc/test-design.md` § 4) installs and upgrades from the drafts
through `LINGGEN_RELEASE_BASE` (`doc/cli.md` § Release override). It builds
a mirror of every draft in the train and serves it to the VM:

```
for r in linggen/linggen linggen/linggen-releases; do
  gh release download <tag-of-r> --repo $r --dir mirror/$r
  gh release view <tag-of-r> --repo $r --json tagName,assets \
    --jq '{tag_name: .tagName, assets: [.assets[] | {name}]}' > mirror/$r/release.json
done
cp linggensite/public/install*.sh mirror/
python3 -m http.server --directory mirror <port>
```

`just release-gate --draft "engine=X.Y.Z app=linggen-vX.Y.Z"` (engine repo,
`scripts/gate/`) does all of it: builds that mirror (a repo left out serves
its latest published release), boots a fresh Tart VM and a clone of
`linggen-prev`, runs the checks, prints a table, exits non-zero on any FAIL.
`--local` gates local builds instead of drafts. Exit 0 is the go to publish;
a green `--draft` run saves its upgraded VM as the next `linggen-prev`.

The plugin bundle comes from the engine draft (`linggen-plugin.tar.gz` +
`.sha256`; a draft without them stops the gate — `--public` too, until a
published engine carries them) or, for `--local`, is packed from this
checkout with `scripts/plugin/package-plugin.sh`. The installers are
not release assets: the mirror takes them from `linggensite/public`; to gate
the release-day ones, `GATE_SITE_PUBLIC=<worktree of the site branch>/public`.
The first install FAILs on any ling-mem binary laid down or asked of the
mirror. The host suites run on
a scratch home under `~/.cache/linggen-gate/host-home`.

`GATE_LING_BIN=target/fast/ling GATE_APP=/nonexistent just release-gate
--local --upgrade-only --skip-host-checks --skip-playbot` gates a `cargo
build --profile fast` binary from `linggen-prev`, without the app, and
publishes nothing. When the baseline holds the same version number, the
upgrade runs from the relabelled manifest (same bytes).

Before the first green 2.3 `--draft` replaces `linggen-prev`, freeze the
2.2.1 baseline (`tart clone linggen-prev linggen-prev-2.2.1`): the takeover
checks need an old ling-mem daemon, and a baseline already on 2.3 has none
(they are a GAP there). Pass it with `--prev linggen-prev-2.2.1`.

A train with a new engine and a new app is gated in ONE run, both drafts
together (`--draft "engine=X.Y.Z app=linggen-vX.Y.Z"`). An engine-only run
upgrades a baseline that already holds the published app, so "an installed
灵境 gets the bundle's code" FAILs falsely (2.2.0, 2026-11-13). The app's
draft takes the unpublished engine by hand: `gh release download X.Y.Z -p
ling-macos-aarch64.tar.gz --repo linggen/linggen`, the binary to
`linggen-app/vendor/ling/darwin-arm64/ling` (fresh inode), `vendor/skills`
checked out at the release commit and committed, then
`./scripts/release.sh linggen --draft --skip-vendor`. Every skills commit
after that means: pointer bump, app draft recut, a new gate run, a new review.

What the 2.2.0 train taught (2026-11-13/14):
- Start with `--rebuild-prev`. A run saves its upgraded VM as `linggen-prev`
  once the VM checks pass, review pending or not; the next run then
  "upgrades" from the release itself, and an old bundle that has `ling.prev`
  inside (the engine keeps it beside its binary) may never launch once moved
  — the run hangs at "an older engine left running is replaced". `ssh` into
  the VM (`tart ip gate-upgrade`, key `~/.cache/linggen-gate/id_ed25519`) to
  see what a silent run waits on.
- One half can be re-run alone: `--upgrade-only --rebuild-prev --skip-playbot
  --skip-host-checks`. Say so in the release record when the evidence is two
  runs.
- Do not let the play bot cross midnight: the day's 问签 pops over the kept
  board and every control under it is a hard finding.
- A stale hand baseline is a hard finding: after a deliberate card or header
  change, `tools/playbot/run.sh --ref HEAD --only baseline --update-baselines`,
  look at the pictures, commit them.
- The screenshot review is the slow loop (one run ≈ 50 min, one review ≈ 8).
  Before the first run, `node lingjing/tools/card-fit.mjs` (skills) must
  report 0, and ask the reviewer for every latent spot it has seen, not only
  the shots that failed.

Before any VM boots, the gate runs `./scripts/check.sh all` and
`./scripts/check.sh e2e` on the engine HEAD it records (a red suite stops it;
`--skip-host-checks` makes that a GAP), and for `--draft` compares HEAD to
the commit the engine draft is cut from (a sha target that differs FAILs; a
branch target — `main`, cut at publish — that has moved WARNs).

It then plays 灵境 on the skills being released (`scripts/gate/playbot.sh`, `just playbot`): the commit
`linggen-app/vendor/skills` holds and the phone pack's (`GATE_PHONE_SKILLS_REF`, else the same). The phone's
灵境 assets are built from it in a scratch worktree (`tool/lingjing_assets.sh`) and its `lingjing_*` tests run — one
that still skips is a GAP. A play-bot hard finding FAILs; its vision review shows REVIEW PENDING until the
reviewer (the deployer Claude via the `playtest` skill, or Hanli) answers the run's `checklist.md` and writes
`review.json` for that run id, then `just release-gate --verdict <run dir>`. `--skip-playbot` is a GAP.

One-time setup, by hand:
- **Gate image** `linggen-gate-base` — without it a quarantined Developer ID
  launch is a GAP (the vanilla image allows App Store apps only, and macOS
  15+ can't switch that from a shell). `tart clone
  ghcr.io/cirruslabs/macos-tahoe-vanilla:latest linggen-gate-base`, `tart run
  linggen-gate-base`, log in (admin/admin), `sudo spctl --global-enable`,
  then System Settings → Privacy & Security → Allow applications from "App
  Store & Known Developers"; shut down. The gate uses it whenever it exists
  (`GATE_IMAGE` overrides).
- **A frozen older baseline** — `linggen-prev` rolls forward, so it only
  tests N-1 → N. Before a green `--draft` replaces it, freeze a copy:
  `tart clone linggen-prev linggen-prev-1.8`; then `--prev linggen-prev-1.8`
  (or `GATE_PREV_VMS`) upgrades it too in the same run, never saved over.

In the VM: `LINGGEN_RELEASE_BASE=http://<host>:<port>` for `install.sh`,
`install-app.sh`, `install-plugin.sh`, `ling update`;
`launchctl setenv LINGGEN_RELEASE_BASE …` before launching Linggen.app.
Rollback check: corrupt one tarball in the mirror → the update is refused
and the old binary still runs; `ling update --rollback` returns to the kept `.prev` — the pre-upgrade
version (the gate puts that binary there when the update kept none), and back.

Takeover (2.3.0 on, `scripts/gate/vm/takeover.sh`): on the old user, rows are
written through the old daemon (three long-term, one staged, one core); after
the upgrade each is its own check — the old daemon gone, nothing listening on
9528, `memory/backups/takeover-<stamp>/` and
`takeover-<version>.done`, `memory/.store.lock` naming the engine's pid, every
row found by id through the engine with its tier, the core row in `ling mem
session-start`, `~/.local/bin/ling-mem` the stand-in and `ling-mem.prev` the
old binary, MCP answering at `9527/mem/mcp` and `9527/mcp/memory`, a
write through the stand-in. Then a rollback — the previous engine and
`ling-mem.prev` put back and started — must find every row, and the release
again must take over without a second backup. `ling update --rollback` to an
engine that does not host memory puts `ling-mem.prev` back over the stand-in
before that engine starts (it starts memory through `ling-mem`): the gate
checks that the old daemon answers on 9528 with every row, including those
written after the upgrade, and that rolling forward takes the store over again.

Plugins: `linggen-prev` (2.2.x) has `linggen@linggen-memory` in Claude Code,
as the builder's own Macs do. After `install-plugin.sh`, first install and
upgrade alike: `linggen@linggen` enabled in Claude Code and Codex, no
`linggen@linggen-memory` in either, Claude Code on this release's hooks, the
stand-in kept, nothing listening on 9528.

Asset pack: after the app's first launch, on first install and upgrade,
灵境's folder lacks the files its `assets.json` lists and the engine holds
them all within 10 min, fetched from assets.linggen.dev (`check_assets_heal`).

## Signing — every Mac asset

A browser download carries the quarantine flag, and Gatekeeper refuses an
ad-hoc build as "damaged". So `ling` and Linggen.app ship
Developer ID signed with the hardened runtime and notarized (the app is
also stapled; a bare binary cannot be, and Gatekeeper checks it online).
Credentials: `linggen-app/secrets/signing.env` (identity + App Store
Connect API key), which the engine's `scripts/sign-mac.sh` also sources. Without the identity those builds fall back to ad-hoc,
and `release.sh` refuses to upload them (`LINGGEN_ADHOC_RELEASE=1` forces
it). Local deploys stay ad-hoc (`codesign -f -s -`). The gate's quarantine
checks FAIL a draft whose asset is not Developer ID signed.

## 0. Preflight — every repo in the train

- `git status` clean and `HEAD == origin/main`. Tags are created at the
  remote head when a draft is published, never before.
- `CHANGELOG.md` entry written and committed.
- The release candidate is already running on Hanli's Mac (deploy-first:
  swap, restart, use it — "on main" is not "tried").
- Engine Linux dry run, no upload:
  `gh workflow run build-linux.yml --repo linggen/linggen -f ref=$(git rev-parse HEAD)`
- Weekend cuts and releases are fine (Hanli 2026-11-07); publishing still waits for his yes.

## 1. Plugin — part of the engine release

The plugin (`plugins/linggen`, `linggen@linggen` from the marketplace files
at this repo's root) has no version or release of its own:
`scripts/sync-version.sh` stamps the engine's version into
`plugins/linggen/VERSION` and both `plugin.json` files (through
`scripts/plugin/build-plugin.sh`), `release.sh` commits them with the stamp,
packs `linggen-plugin.tar.gz` (+ `.sha256`) with
`scripts/plugin/package-plugin.sh` and attaches both to the engine release.
`linggen.dev/install-plugin.sh` installs that bundle when there is no git, and
retires an old `linggen@linggen-memory`. Before a release that touched the
plugin: `bash scripts/plugin/plugin-check.sh` (fails when the versions differ)
and `plugins/linggen/hooks/tests/*.test.sh`.

## 2. Engine

Repo `linggen/linggen`, tag `X.Y.Z` (no `v`). `Cargo.toml`, `Cargo.lock`,
`ui/package.json` and the plugin's `VERSION` + both `plugin.json` are stamped
by the script — do not bump by hand.

1. `CHANGELOG.md` entry; commit, push.
2. `./scripts/release.sh X.Y.Z --draft` — builds the UI and `ling`, commits
   and pushes `chore: release X.Y.Z`, uploads `ling-macos-aarch64.tar.gz` and
   `linggen-plugin.tar.gz` + `.sha256`.
3. `./scripts/release.sh X.Y.Z --linux-ci` — `ling-linux-x86_64` +
   `ling-linux-aarch64` onto the draft (~10 min).
4. `./scripts/release.sh X.Y.Z --manifest-only` — `manifest.json` lists all
   three. No macos-x86_64 target.
5. Local swap: `lsof -ti :9527 | xargs kill` (linggen-shell holds the port
   too); `rm` + `cp` + `codesign -f -s -` into `/usr/local/bin/ling` AND
   `/Applications/Linggen.app/Contents/MacOS/ling` (the bundle is TCC-blocked:
   stage in /tmp, Finder-mediated copy); restart `ling --web`.
6. After the release gate passes:
   `gh release edit X.Y.Z --draft=false --latest --repo linggen/linggen`.
7. Verify from outside: `curl -fsSL https://linggen.dev/install.sh | bash`
   in a scratch dir → `ling --version`.

## 3. Skills — no release

The engine downloads `linggen/skills` `main` (`extensions/marketplace.rs`).
Only check: everything is pushed.

## 4. Mac app — after the engine

Repo `linggen-app`; releases on `linggen/linggen-releases` as `linggen-vX.Y.Z`.

1. `apps/linggen/app.toml` version; commit.
2. `./scripts/release.sh linggen --draft` — refreshes `vendor/ling`
   (`download-ling.sh`, latest engine release) and `vendor/skills` (`main`),
   builds signed (Developer ID, hardened runtime,
   `scripts/entitlements.plist`), notarizes + staples, uploads DMG + tarball
   + `.sha256` to a draft. Never `--tarball` for a release: it ships unsigned,
   and a browser download is refused as "damaged".
3. Verify in the bundle: `Contents/MacOS/ling --version` is the engine just
   cut; the skills are present.
4. After the release gate passes:
   `gh release edit linggen-vX.Y.Z --draft=false --latest --repo linggen/linggen-releases`.

Version lines are independent — Linggen 0.x, CFO, Mac Shifu, engine 1.x.
Do not align them.

## 5. iOS — only when the wire changed

Rebuild only if the engine changed `/api` routes, RTC frame shapes, or topic
payloads the phone reads. Then: `pubspec.yaml` `1.0.0+N`, archive, App Store
Connect, submit. Approval alone ships nothing — set availability.

## 6. Website — nothing

`install.sh` reads releases/latest, `/dl` proxies GitHub, `release-tags.js`
syncs at build. Update `guide.md` only when screens changed.

## 7. README — only if commands or features changed

No README carries a version.

## 8. Announce

Direct `apps.apple.com` link, never "search for it". HN and Reddit text is
hand-written.
