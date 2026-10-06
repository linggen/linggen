# Release checklist

One train, in order: ling-mem → engine → Mac app. Each step names the script
that does it; the script is the truth, this page is the order.

## Release gate — between each draft and its publish

The gate (`doc/test-design.md` § 4) installs and upgrades from the drafts
through `LINGGEN_RELEASE_BASE` (`doc/cli.md` § Release override). It builds
a mirror of every draft in the train and serves it to the VM:

```
for r in linggen/linggen linggen/linggen-memory linggen/linggen-releases; do
  gh release download <tag-of-r> --repo $r --dir mirror/$r
  gh release view <tag-of-r> --repo $r --json tagName,assets \
    --jq '{tag_name: .tagName, assets: [.assets[] | {name}]}' > mirror/$r/release.json
done
cp linggen-memory/plugins/linggen/scripts/install-bin.sh linggensite/public/install-plugin.sh mirror/
python3 -m http.server --directory mirror <port>
```

`just release-gate --draft "engine=X.Y.Z mem=vX.Y.Z app=linggen-vX.Y.Z"`
(engine repo, `scripts/gate/`) does all of it: builds that mirror (a repo
left out serves its latest published release), boots a fresh Tart VM and a
clone of `linggen-prev`, runs the checks, prints a table, exits non-zero on
any FAIL. `--local` gates local builds instead of drafts. Exit 0 is the go to
publish; a green `--draft` run saves its upgraded VM as the next
`linggen-prev`. The app's draft bundles the latest *published* engine
(`download-ling.sh`), so a train with a new engine gates engine + ling-mem,
publishes them, then cuts and gates the app.

Before any VM boots, the gate runs `./scripts/check.sh all` and
`./scripts/check.sh e2e` on the engine HEAD it records (a red suite stops it;
`--skip-host-checks` makes that a GAP), and for `--draft` compares HEAD to
the commit the engine draft is cut from (a sha target that differs FAILs; a
branch target — `main`, cut at publish — that has moved WARNs).

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
`install-app.sh`, `ling update`, `ling-mem upgrade --yes`;
`launchctl setenv LINGGEN_RELEASE_BASE …` before launching Linggen.app.
Rollback check: corrupt one tarball in the mirror → the update is refused
and the old binary still runs; `ling update --rollback` /
`ling-mem upgrade --rollback` return to the kept `.prev` — the pre-upgrade
version (the gate puts that binary there when the update kept none), and back.

## Signing — every Mac asset

A browser download carries the quarantine flag, and Gatekeeper refuses an
ad-hoc build as "damaged". So `ling`, `ling-mem` and Linggen.app ship
Developer ID signed with the hardened runtime and notarized (the app is
also stapled; a bare binary cannot be, and Gatekeeper checks it online).
Credentials: `linggen-app/secrets/signing.env` (identity + App Store
Connect API key), which `scripts/sign-mac.sh` in the engine and ling-mem
repos also sources. Without the identity those builds fall back to ad-hoc,
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
- No weekend cuts.

## 1. ling-mem — only when `src/` changed since the last `v*` tag

Repo `linggen/linggen-memory`, tag `vX.Y.Z`.

1. `Cargo.toml` version + `CHANGELOG.md`; commit, push.
2. `./scripts/release.sh vX.Y.Z --draft` — mac aarch64 tarball + `.sha256`
   on a draft.
3. `gh workflow run build-linux.yml --repo linggen/linggen-memory -f ref=<sha> -f release_tag=vX.Y.Z`
   — both Linux tarballs land on the draft (~10 min).
4. Local swap: kill the 9528 daemon; `rm` + `cp` into `~/.local/bin/ling-mem`
   (fresh inode, never overwrite in place), `codesign -f -s -`, keep the old
   binary aside, `ling-mem serve --port 9528`.
5. After the release gate passes:
   `gh release edit vX.Y.Z --draft=false --latest --repo linggen/linggen-memory`
   — eight assets (the six binaries + `linggen-plugin.tar.gz` and its
   `.sha256`, which `install-plugin.sh` needs). Publishing is the distribution: every host resolves `^1`.

Plugin bundles bump only when hooks or `SKILL.md` changed, independent of the
binary: `plugins/linggen/.claude-plugin/plugin.json`,
`plugins/linggen/.codex-plugin/plugin.json`,
`plugins/openclaw/openclaw.plugin.json`; ClawHub via `clawhub skill publish`
(versions are immutable). Where each channel pins:
`linggen-memory/doc/release-targets.md`.

## 2. Engine

Repo `linggen/linggen`, tag `X.Y.Z` (no `v`). `Cargo.toml`, `Cargo.lock`
and `ui/package.json` are stamped by the script — do not bump by hand.

1. `CHANGELOG.md` entry; commit, push.
2. `./scripts/release.sh X.Y.Z --draft` — builds the UI and `ling`, commits
   and pushes `chore: release X.Y.Z`, uploads `ling-macos-aarch64.tar.gz`.
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
