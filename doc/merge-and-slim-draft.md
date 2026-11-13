# Merge ling-mem into ling, and slim the app — discussion draft (2026-11-13)

**Status: talk only, nothing decided, nothing built.** Hanli and Claude,
2026-11-13. To be picked up the week of 2026-11-16.

## 1. Hanli's words

「我开始把ling-mem单独分出去一个bin是为了以后方便linggen用户切换使用不同的memory
系统. 但现在看, ling-mem和linggen似乎应该是在一个bin里. 它俩几乎不可分割.
我们有可能把两个repo合并成一个, bin也合并成一个9527. 现在linggen app大概242 MB,
ling-mem多大? 是否可以把模型, 权重拿出去, 发布到CF上, 安装合并, 瘦身后的linggen
bin, 第一次启动在后台下载?」

## 2. Measured 2026-11-13 (his Mac, installed 2.1.0)

| Piece | Size | Note |
|---|---|---|
| `Linggen.app` on disk | **493 MB** | includes a 133 MB `ling.prev` (§3.1) |
| `Contents/MacOS/ling` | 133 MB | embeds `ui/dist` |
| ↳ `ui/dist` | 86 MB | `yinyue.vrm` 16 MB + dozens of `.fbx` animations, 2–3 MB each |
| `Contents/MacOS/linggen-shell` | 10 MB | |
| `skills/lingjing` | 209 MB | `worlds/jiuding/art` 139 MB (1,000 webp), `scripts` 43 MB |
| other skills (cfo, dj, apple-shifu, shared-memory) | ~5 MB | |
| `~/.local/bin/ling-mem` | **114 MB** | separate binary, not in the app |

**Model weights are already not bundled** — each is downloaded on first use:
embedding (fastembed, `~/.linggen/cache/fastembed`) 87 MB; Qwen3-TTS 1.6–2.3 GB,
Qwen3-ASR 2.3 GB, FLUX 4.3 GB (`~/.linggen/models/hf-hub`); Kokoro voices 1 MB.
The binaries' weight is code: candle, lance/arrow, fastembed.

## 3. Slimming, biggest win first

1. **`ling.prev` inside the bundle — 133 MB, a bug.** `ling restart` keeps the
   previous build for rollback beside the binary, so the app doubles and its
   code signature may break. Keep it under `~/.linggen/` instead. Independent of
   everything else; fix first.
2. **灵境 art, ~180 MB → CF R2**, downloaded in the background per world or
   chapter. The first chapter's art stays in the package so play starts
   offline at once.
3. **Yinyue's model + animations, ~60 MB** → downloaded when the user first
   meets her. She is already hidden until met (`met_when`), so that is the
   natural moment.
4. **Merging the binaries** — both carry tokio, candle (fastembed's qwen3 and
   Kokoro TTS) and the HTTP/MCP stack. Estimate: merged ≈ 170–190 MB instead of
   133 + 114. An estimate; a test build gives the real number.

After 1–3: a download around 80–100 MB, or ~150–200 MB with the merged binary,
everything heavy fetched in the background. The first-launch download needs an
offline fallback and to work from China (the 1.7.0 China-ready install lesson).

### Where downloads come from today, and what goes on CF

Hanli: 「第一次启动时下载的模型和权重是从哪里下载的? 我们应该发布到CF上吗?」

Each is fetched when its feature is first used, not at first launch:

| Model | Size | Source today |
|---|---|---|
| Qwen3-Embedding-0.6B (ling-mem) | ~1.2 GB | huggingface.co → ModelScope (`linggen-memory/src/embed/model_fetch.rs`), SHA-256 pinned |
| Qwen3-TTS, Qwen3-ASR, FLUX.2 klein 4B (engine, MLX) | 1.6–4.3 GB each | huggingface.co → hf-mirror.com (`linggen/src/hf_mirror.rs`; a user `HF_ENDPOINT` wins); **not pinned** |
| Kokoro voices | 1 MB | Hugging Face |
| engine, skills, ling-mem binaries | — | GitHub releases → linggen.dev `/dl` (CF) |

Claude's recommendation:

- **Models stay on Hugging Face as the source.** Free, official, upstream owns
  updates and licences; China is already covered by ModelScope and hf-mirror,
  and CF is itself patchy inside China.
- **One exception: the embedding model gets linggen.dev R2 as a last
  fallback**, under the same pins — memory is dead without it, while voice,
  ASR and pictures are optional features.
- **Our own assets must go on R2** (灵境 art, Yinyue's model + animations) — no
  one else hosts them — each file SHA-256 pinned.
- **Pin TTS, ASR and FLUX** (hashes, or at least an HF revision): hf-mirror.com
  is a third party and today could hand over altered files unnoticed.
- **Check FLUX.2 klein's licence** before ever hosting it ourselves; Qwen3 and
  Kokoro are Apache-2.0.

## 4. Merging: for, and the two constraints

For: one process, one port, no cross-process calls, one version and one
release, no more "ling-mem is down so memory goes quiet".

Constraints to keep:

- **Memory without Linggen.** The positioning is "one memory, shared by Claude
  Code, Codex, OpenClaw and Linggen" (2026-09-25); a Claude Code user without
  Linggen still needs it. So the merged binary needs a **memory-only mode**
  (e.g. `ling mem serve`) that starts no apps, Yinyue or UI. Distributed-memory
  replicas on another machine would run in it too.
- **Port and plugin config.** The CC plugin, Codex and OpenClaw point at
  `9528/mcp` today; after the merge it becomes `9527/mcp/memory`. Old clients
  get a forwarding period, or `install.sh` rewrites their config.

Given up: swapping in another memory system as a separate binary. If that is
still wanted, keep memory a module behind a fixed interface in the code —
cheaper than a second binary.

Repos: `linggen` + `linggen-memory` into one. Not looked at yet: history
(subtree vs fresh), the plugin/marketplace repo (`linggen/linggen-memory` is the
Claude Code marketplace name), CI and the release scripts.

## 5. Claude's recommendation

- **Slim first (§3 items 1–3)** — no architecture change, the biggest win.
- **Merge as its own project** — first a test merged build for the real size,
  then decide.

## 6. Open, for next week

- **Is a memory-only mode a hard requirement?** If ling-mem without Linggen no
  longer matters, the merge is simpler: no second mode, the port just switches.
- Where 灵境 art lives on R2 (per world? per chapter?) and how updates to it ship.
- Whether the release gate (`test-design.md`) must test a first launch with no
  network.
