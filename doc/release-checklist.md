# Release checklist

One train, in order: ling-mem → engine → Mac app. Each step names the script
that does it; the script is the truth, this page is the order.

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
5. `gh release edit vX.Y.Z --draft=false --latest --repo linggen/linggen-memory`
   — six assets. Publishing is the distribution: every host resolves `^1`.

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
6. `gh release edit X.Y.Z --draft=false --latest --repo linggen/linggen`.
7. Verify from outside: `curl -fsSL https://linggen.dev/install.sh | bash`
   in a scratch dir → `ling --version`.

## 3. Skills — no release

The engine downloads `linggen/skills` `main` (`extensions/marketplace.rs`).
Only check: everything is pushed.

## 4. Mac app — after the engine

Repo `linggen-app`; releases on `linggen/linggen-releases` as `linggen-vX.Y.Z`.

1. `apps/linggen/app.toml` version; commit.
2. `./scripts/release.sh linggen --tarball` — refreshes `vendor/ling`
   (`download-ling.sh`, latest engine release) and `vendor/skills` (`main`),
   builds, uploads a draft.
3. Verify in the bundle: `Contents/MacOS/ling --version` is the engine just
   cut; the skills are present.
4. `gh release edit linggen-vX.Y.Z --draft=false --latest --repo linggen/linggen-releases`.

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
