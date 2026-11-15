# Architecture review — before the merge and the crate split

**Status: step 0 of `plan-3.0.md`, read-only, 2026-11-15.** Written for Hanli.
Engine branch `doc/arch-review` at `4251bce4` (2.2.1 + the plan); memory repo
`linggen-memory` at `5095569` (ling-mem 1.13.0).

Tags: **(code)** = read in code, with the place; **(inferred)** = my
reading, not proven by a build or a run. Nothing here was built or run.
Paths are relative to the engine repo, or start with `linggen-memory/`.

Caveat on the module map: its counts are `crate::x` paths only. `super::`
paths and nested `use crate::{a, b}` groups are invisible to it, so the
real coupling is somewhat larger than the counts (inferred).

---

## 1. The engine today

One crate, `linggen` (bin `ling`), 122,740 lines, 25 top-level modules
(`src/main.rs` declares them all as `mod`).

### 1.1 Layers as they actually are

| Layer | Modules | Lines | Depends on (code, module map) |
|---|---|---|---|
| Leaves | `message`, `language`, `mirror` | 625 | nothing |
| Base | `paths`, `util`, `reach`, `hf_mirror`, `runtime`, `js_runtime`, `logging`, `credentials`, `telemetry` | 4.3k | each other; **`paths` → `engine`, `credentials` → `config`** |
| Model | `config`, `provider`, `account`, `mcp_client`, `prompts` | 14.2k | base; **config → engine, provider → engine, account → engine, mcp_client → engine, prompts → engine** |
| Store | `state_fs`, `perception`, `senses`, `pictures` | 6.3k | base, model; **state_fs → engine, perception → engine** |
| Catalog | `extensions` (skills, agents, marketplace, missions) | 7.1k | everything below, **and engine (49 refs)** |
| Core | `engine` | 39.9k | everything below, **and extensions (17), server (3)** |
| Front | `server`, `cli`, `main` | 50.8k | everything |

The direction is clean **except** for a handful of items reaching *up*.
Every cycle below is caused by a few named items, not by a tangle (code).

### 1.2 Cycles and the items that cause them

| Cycle | Upward items (file → item) | Kind |
|---|---|---|
| `engine` ↔ `server` | `engine/context.rs:632` → `server::commit_compaction` (defined `server/chat/compact_rows.rs:325`); `engine/tools/builtin/claude_session.rs:62,74` → `server::claude::supports_model`, `server::claude::new_meta` (`server/claude/mod.rs:175,210`) | 3 call sites |
| `engine` ↔ `extensions` | engine → `extensions::skills::parse_skill_text`, `extensions::agents::parse_agent_markdown`, `extensions::frontmatter::split` (`engine/skill/record.rs`, `engine/skill/tools.rs`, `engine/agent/met.rs`, `engine/prompt/place.rs`, `engine/tools/builtin/app_tool.rs`, plus tests in `engine/mod.rs`, `engine/prompt/persona_tests.rs`, `engine/light_turn/tests.rs`). extensions → `engine::skill::{Skill, SkillRegistry, NoticeDecl, QueueMode, QuestsConfig, params, record::Places}`, `engine::skill_tool::SkillToolDef`, `engine::mission::record::Mission`, `engine::permission::apply_grants`, `engine::tool_scope::compute_tool_scope`, `engine::tools::{ToolResult, PendingAskUser, AskUserBridge, DEFAULT_MAX_TOOL_OUTPUT_BYTES, memory_http}`, `engine::StreamedToken`, `engine::agent::AgentManager` (missions scheduler) | Skill *types* live in `engine/skill/`, skill *parsing* in `extensions/`; missions *run* agents |
| `config` ↔ `provider` | config → `provider::models::{LINGGEN_CLOUD_MODEL_ID, CHATGPT_BUILTIN_MODEL_IDS, chatgpt_successor, …}`, `provider::set_first_event_timeout` (`config.rs:476-704`); provider → `config::ModelConfig` (`provider/models/{mod,catalog,providers}.rs`) | model-id migration inside config load |
| `config` → `engine` | `engine::permission::PermissionMode` (`config.rs:283,376`) | one enum |
| `config` → `state_fs` → `provider` → `config` | `config.rs:489` → `state_fs::client::resolve_ling_mem`; `state_fs/devices/mod.rs:113,146` → provider model ids | ling-mem address (goes away with the merge) |
| `paths` → `engine` | `paths.rs:121` → `engine::tools::search_exec_find_git_root` | one helper |
| `state_fs` → `engine` | `engine::agent::LEAD_AGENT_ID`, `engine::tools::search_exec_find_git_root` | a constant, a helper |
| `perception` → `engine` | `engine::agent::COMPANION_AGENT_ID` | a constant |
| `provider` → `engine` | `provider/openai/citations.rs:100` → `engine::answer_refs::is_ref_id` | one predicate |
| `mcp_client`, `prompts` → `engine` | `engine::tools::json_schema::oai_tool_definitions` — **test code only** (`mcp_client/registry.rs:554` after `#[cfg(test)]` at :392; `prompts.rs:265,327` after :241) | tests |
| `provider` ↔ `pictures` | provider → `pictures::kind::media_type_of_base64` (`provider/anthropic.rs:441`, `provider/openai/{wire,responses_api}.rs`); pictures → `provider::{codex_auth, openai::OpenAiClient, models}` (`pictures/chatgpt.rs`) | one pure helper (`pictures/kind.rs:90`, imports only `base64`) |
| `credentials` → `config` | `credentials.rs:1` → `config::ModelConfig` | one type |
| `extensions` → agent types | `extensions/agents/mod.rs:13-14` → `engine::agent::record::{AgentSpec, AgentSpecFile}`, `engine::agent::registry::AgentRegistry`; missions → `engine::mission::registry::MissionRegistry`, `record::MissionPermission`, `engine::agent::AgentRunStatus`, `engine::ActiveMission`, `engine::events` | records + registry traits of all three extension types |
| `mcp_client` → `state_fs` → … | `mcp_client/config.rs:117` → `state_fs::client::{token, DEVICE_HEADER}` | client.json |
| `account` ↔ `engine` | `account/cloud_save.rs:28` → `engine::skill::{Skill, SkillSource}`; engine → `account::{resolve_token, site_url, cloud_save::*, cloud::*}` (`engine/cloud_meter.rs`, `engine/skill/tools.rs`, `engine/skill_tool.rs`, `engine/web_search.rs`) | game saves are per skill |
| `account` → `extensions` | `account/cloud_save.rs:521-567` — test code only (`#[cfg(test)]` at line 475) | tests |
| `state_fs` → `perception`, `pictures` | `state_fs/devices/mod.rs:190`, `state_fs/session_images.rs:103` | same-layer, fine if they share a crate |

`util` → `runtime` is one-way (`runtime.rs:300` is a doc comment) (code).

### 1.3 Seams that already exist (code)

- `AgentManager::new` takes `Arc<dyn SkillRegistry>`, `Arc<dyn AgentRegistry>`,
  `Arc<dyn MissionRegistry>` (`engine/agent/mod.rs:319`); `engine/test_registries.rs`
  has empty ones. The *registry* direction is already inverted — only
  parsing and types still cross.
- `BrowserBridge` trait (`engine/tools/browser_tool.rs:27`), served by the
  server's bridge hub — the same pattern can carry the two server items above.

---

## 2. Target crate layout

Seven crates in one Cargo workspace (`crates/<name>`), memory first.

### 2.1 Crates, bottom up (acyclic)

| # | Crate | What moves in | ≈ lines | Depends on |
|---|---|---|---|---|
| 1 | `linggen-memory` | `linggen-memory/src/` (lib `ling_mem` today), minus §3b deletions | 18–20k | none of ours (lancedb, candle, fastembed) |
| 2 | `linggen-base` | `paths`, `util`, `message`, `language`, `mirror`, `reach`, `hf_mirror`, `runtime`, `js_runtime`, `logging`, `telemetry`; + `find_git_root`/`project_ancestors`, agent-id constants, the answer-ref id grammar (`is_ref_id`), `pictures::kind` | 4.4k | — |
| 3 | `linggen-model` | `config`, `credentials`, `provider`, `account` (minus `cloud_save`), `mcp_client`, `prompts`, `state_fs::client` (token, device id); + `PermissionMode` | 15k | base |
| 4 | `linggen-store` | `state_fs` (minus `client`), `perception`, `senses`, `pictures` (minus `kind`) | 6.2k | base, model |
| 5 | `linggen-skills` (the catalog) | records + registry traits of all three extension types (`engine/skill`, `engine/agent/{record,registry}`, `engine/mission/{record,registry}`), `engine/skill_tool` defs, `extensions/{frontmatter, includes, skills, agents, marketplace}`, `account/cloud_save` | 11–12k | base, model, store |
| 6 | `linggen-engine` | the rest of `engine/`, `extensions/{missions, script}`, the in-process memory facade | 42k | 1–5 |
| 7 | `linggen` (bin `ling`) | `server/`, `cli/`, `main.rs`, `ui/dist` embed | 51k | 1–6 |

### 2.2 What must move or be inverted, per cycle

| Cycle | Fix | Size |
|---|---|---|
| engine ↔ server | Move `commit_compaction` from `server/chat/compact_rows.rs` to `engine/context` — it rewrites `Vec<ChatMsg>` but drags `summary_cut`, `COMPACTION_SENDER`, `thread::since_compaction` and uses `AgentEngine::truncate_to_tokens` (code), so the row-rewrite half of `compact_rows.rs` moves, the driver half stays; move `claude::supports_model` + `new_meta` down to engine (they read `ModelConfig` / build `SessionMeta`), or move the `claude_session` tool up into server | 3 fns |
| engine ↔ extensions | Put the three record/registry pairs and their parsers in one crate (`linggen-skills`); `extensions/missions` (runs agents via `AgentManager`, calls `memory_http`) and `extensions/script.rs` (uses `ToolResult`, `AskUserBridge`) go **up** into `linggen-engine`; `apply_grants` and `compute_tool_scope` calls stay in engine, called on the parsed skill | biggest |
| config ↔ provider | Same crate (`linggen-model`); a cycle inside a crate is legal | 0 |
| config → engine | `PermissionMode` → `linggen-model` (re-exported from `engine::permission`) | 1 enum |
| config/state_fs client | `state_fs::client` → model; `resolve_ling_mem` is deleted by the merge (§5) | small |
| paths, state_fs → engine | `find_git_root`, `project_ancestors_within`, `home_forms` out of `engine/tools/search_exec.rs` → base — pure path walks on `dirs` (code) | 4 fns |
| state_fs, perception → engine | `LEAD_AGENT_ID`, `COMPANION_AGENT_ID` → base | 2 consts |
| provider → engine | `is_ref_id` + `parse_id`, `Kind`, `KINDS`, `split_digits` (`engine/answer_refs/mod.rs:141-`) → base | id grammar |
| provider ↔ pictures | `pictures/kind.rs` → base | 1 file |
| credentials → config | `credentials` joins `linggen-model` | 0 |
| mcp_client, prompts → engine | test-only: move those two tests into the engine crate. `json_schema` itself stays — it lists the built-in registry (`super::builtin`) and MCP registry (code) | 2 tests |
| account ↔ engine | `account/cloud_save` → `linggen-skills` (it is per-skill save sync) | 2.3k lines |

Mechanical cost the map does not show (code): `server/` has 531 and `engine/`
223 `pub(crate)` items; every one used across a new crate line becomes `pub`.
`ChatRequest` and most server types are `pub(crate)` (`server/chat/types.rs`).
Server tests use `engine::test_registries` (`server/background.rs:649`) →
needs a `test-support` feature on the engine crate. `ts-rs` exports
(`ui/src/types/generated/`) need their export path set per crate (inferred).

### 2.3 Order of splits

Peel the **top** first: the cut between front and core is three call sites
and buys the most; the bottom crates need more small moves for less gain.

| Step | Split | Moves first | Size | Risk | Rebuild gain (inferred) |
|---|---|---|---|---|---|
| 3.1 | `linggen-memory` as a workspace member (path dep, history imported) | lock file pinned (fastembed 5.13.4); tokio `full`, `rand` 0.9/0.10, `dirs` 5/6 duplicates accepted for now | 23k moved, 0 engine lines | medium (deps) | memory's 109 MB of deps compile once and stay cached; a memory edit recompiles memory + engine + bin |
| 3.2 | `linggen` (bin) ⟂ `linggen-engine` (everything else, still one crate) | §2.2 row 1; `pub(crate)` → `pub` across the line | 51k / 72k | medium (visibility churn, ts-rs, tests) | **largest**: a server/cli edit (the most edited area) recompiles 51k, not 123k |
| 3.3 | `linggen-base` | rows paths/state_fs/perception/provider/pictures-kind; two tests | 4.4k | low | small; enables 3.4 |
| 3.4 | `linggen-model` + `linggen-store` | `PermissionMode`, `state_fs::client`, `credentials` | 21k | low–medium | an engine edit no longer re-checks 21k lines |
| 3.5 | `linggen-skills` | three record/registry pairs + parsers together; missions/script up; `cloud_save` | 11k | **high** (most crossings, 1.4k-line `extensions/skills/mod.rs`) | modest; mostly a boundary win |

Note: `linggen-memory/CLAUDE.md` says "single-crate discipline, don't
re-introduce a workspace". The plan moves that code into the engine repo, so
the rule is read as "memory stays one crate", not "no workspace" (inferred).

Each step is one commit of moves only (plan rule 2); `cargo check` per step.
Stop after 3.2 if time is short — it holds most of the build gain (inferred).

---

## 3. The merge

### 3a. Engine code that talks to ling-mem (code: grep `ling_mem|ling-mem|9528|memory_http`, 39 files)

| Group | Where | 2.3 — no visible change | Eventual (in-process) |
|---|---|---|---|
| HTTP client for programs | `engine/tools/memory_http.rs` (`call_memory_http`, `post_memory_verb`); callers `server/app_pages.rs:113` (skill page capability), `server/mcp.rs:650` (dream rollup), `extensions/missions/scheduler.rs:1031` (stats), `engine/prompt/core_block.rs` (session start / core rows) | same function, base URL = engine's own memory facade | `memory::Service` handle called directly; no JSON over HTTP |
| Model tools (MCP client) | `mcp_client/config.rs` `with_builtin` / `BUILTIN_MEMORY` (`url/mcp`); `server/background.rs:197`; `server/claude/runtime.rs:191,218` (Claude SDK sessions get the same server); `engine/tools/memory_mcp.rs` (argument shaping: host, source_session, cwd/scope, user-voice guard) | builtin entry points at `127.0.0.1:9527` memory MCP | a native tool provider with ling-mem's schemas; `memory_mcp` shaping stays, applied in-process |
| Phone door | `server/api/memory.rs` `passthrough` + `stamp_person` / `restamp_device`, route `/api/memory/{verb}` (`server/routes/settings.rs:57`) | unchanged, forwards in-process | one `/api/memory/{verb}` handler (§3c) |
| Daemon ownership | `memory_http.rs` `autostart`, `install_ling_mem`, `resolve_ling_mem`, `ling_mem_version`, pin `^1`, `INSTALL_BIN_URL`; `server/background.rs:87-107` pre-warm; `watch_for_late_servers` (`:207`) | **deleted** | — |
| Connect / primary / replica | `server/api/ling_mem_connect.rs` (relay pending/decide), `ling_mem_primary.rs` (relay role), `ling_mem_client.rs` (point this engine at another machine's ling-mem, request access, save token); routes `server/routes/devices.rs:55-79`; UI `LingMemConnectModal.tsx`, `settings/LingMemPrimary.tsx`, `settings/LingMemAddress.tsx` | relays call memory in-process | connect + primary fold into the engine's device pages; "another machine's address" — Q1 |
| Config | `config.rs:301-353` `ling_mem_url`, `DEFAULT_LING_MEM_PORT`, `$LING_MEM_URL`; `state_fs/client.rs` `ling_mem`, `resolve_ling_mem`; `engine/types.rs:177-209`; `AgentManager::set_ling_mem_url` (`engine/agent/mod.rs:827`) | kept, defaults to self | deleted with Q1 |
| Status | `cli/status.rs:81-111` `check_ling_mem` (binary on PATH, version, `/api/health`) | report in-process store | same |
| Missions and dream | `scheduler.rs:642` `backup_store_best_effort`; `server/mcp.rs` `memory_dream_status` / `memory_dream_run` (compose rollup + engine run state); ling-mem `http/lease.rs` (dream lease) | unchanged | backup and lease become direct calls |
| Telemetry | `telemetry/imp.rs:260,369` (`LING_MEM_NO_TELEMETRY`, sibling detection) | unchanged | one telemetry module (§3b) |
| Tests self-gating on 9528 | `mcp_client/registry.rs:452-579`, `mcp_client/client.rs:209-222` | repoint | rewrite against the in-process store |
| Strings only | `engine/permission/mod.rs` tests (`ling-mem add/list` shell lines), comments elsewhere | — | rename with `ling mem` |

`linggen-mobile` has no direct use of 9528: its references are comments; it
calls the Mac's `/api/memory/<verb>` over the data channel
(`lib/services/yinyue/mac_memory.dart`) (code).

### 3b. linggen-memory/src, module by module

| Module | Lines | Verdict | Note |
|---|---|---|---|
| `memory/` (store, schema, scope, hybrid, recall, changes, backups, maintenance, schema_version) | 6.9k | **keep as is** | the product |
| `embed/` (Qwen3 via fastembed, `LoadOnce`) | 0.6k | keep; `model_fetch.rs` folds into engine `hf_mirror.rs` | lazy load already (`embed/load_once.rs`) |
| `replica/` | 1.1k | keep | primary/replica stays a memory concern |
| `hash.rs` | 0.2k | keep | |
| `http/memory.rs`, `days`, `chains`, `issues`, `stats`, `session`, `sync`, `lease`, `migration`, `config`, `primary`, `envelope`, `state` | 5.3k | keep, **mounted in the engine router** | route clashes below |
| `http/mcp.rs` | 0.9k | fold into the engine's MCP door (§3d); drop its self-POST to `127.0.0.1:<port>` (`http/mcp.rs:586`) | |
| `http/connect/` | 1.0k | fold into engine pairing (§3c) | |
| `http/gate.rs`, `http/local.rs`, `http/network.rs`, `http/health.rs` | 0.6k | **delete**: engine `lan_gate` + `loopback_guard` + `[server] host` + its `/api/health` | |
| `http/presence.rs` | 0.1k | fold into engine device "last seen" | |
| `http/ui.rs` + `static/` (console, 4.2k lines JS/CSS/HTML) | — | **delete** if the `shared-memory` skill page covers it (Q2) | clashes with engine `/`, `/assets/*` |
| `daemon/lifecycle.rs`, `pidfile.rs`, `serve.rs`, `mod.rs` | 1.1k | **delete**; keep the start-up steps of `serve::run` (open stores, attach change log, start embedder load) as `memory::open()` | `daemon/maintenance.rs` (0.2k) keep as a background task |
| `update/` | 0.9k | **delete** — engine `cli/self_update.rs` | |
| `telemetry/` | 0.6k | **delete** — fold verb counting into engine `telemetry/` | |
| `cli/mod.rs`, `cli/client.rs` | 2.8k | keep data verbs as `ling mem …`; delete lifecycle verbs (serve/start/stop/restart/upgrade) | §3e |
| `main.rs` | 32 | delete | |

Route clashes when mounted at the same paths on 9527 (code: route lists of
both): `/`, `/assets/*`, `/api/health`, `/api/config`, `/mcp`,
`/api/memory/*`. Engine's `/api/memory/{verb}` is the stamped phone door;
ling-mem's is the raw door. Everything else (`/api/connect/*`, `/api/sync/*`,
`/api/network`, `/api/schema/apply`) is free.

### 3c. Two trust systems

| | Engine pairing | ling-mem connect |
|---|---|---|
| For | a phone (and app webviews) | another machine's ling-mem / agents |
| Ceremony | 6-digit code on the Mac's screen typed on the phone; QR (`server/api/pair/`) | the machine asks, someone at the Mac clicks Allow/Deny (`linggen-memory/src/http/connect/`; UI relayed via `server/api/ling_mem_connect.rs`) |
| Token store | `~/.linggen/paired-devices.json` | **the same file** (`connect/devices.rs::allow`) |
| Gate | `server/mod.rs::lan_gate`: header, Bearer **or cookie**; plus `loopback_rejection` (Host/Origin rebinding guard) | `http/gate.rs::lan_gate`: header only; no rebinding guard; records presence |
| Row | phone row may carry `account` | machine row: no `account` |
| Off-LAN | WebRTC relay (`server/rtc/`) | none |

So there is already **one token list and two ceremonies and two gates** (code).
Simplest single system: one list, one gate (the engine's), two ceremonies kept
as UI, and **a `kind` on each row** (`phone` | `machine`).

Why the kind is needed (code): the engine's memory door decides whose rows a
caller sees from its pairing row (`server/api/memory.rs::person_for`), and a
machine approved through connect has no `account`. The memory daemon's own
door applies a different rule. With one handler on 9527 there is one rule:
loopback → owner; `machine` → owner (it is the owner's other computer);
`phone` → its own person, as today. The details of today's two rules are kept
out of this public file.

### 3d. Two MCP doors → one

| Today (code) | After |
|---|---|
| `127.0.0.1:9527/mcp` — `server/mcp.rs`: `browser_*`, `x_*`, `agent_*`, `memory_dream_status/run`, issues | one `/mcp` on 9527 serving both tables |
| `127.0.0.1:9528/mcp` — `linggen-memory/src/http/mcp.rs`: `memory_*` (narrow schemas), self-POSTs to its own `/api/memory/*` | `memory_*` dispatch in-process |
| Plugins ship two MCP entries; front-door instructions say "add ling-mem at 9528" (`server/mcp.rs:419`) | plugins ship one entry; instruction string rewritten |

The engine's **own** sessions must not see the external door's
`browser_*`/`agent_*` tools as MCP: 2.3 points `with_builtin` at a
memory-only MCP path (e.g. `/mcp/memory`) on 9527; the eventual step replaces
it with a native provider (inferred). Two identical `memory_search` tools must
never reach one model (`engine/tools/memory_mcp.rs` header) — keep that rule.

### 3e. Risks and mitigations

| Risk | What the code shows | Mitigation |
|---|---|---|
| Blocking the async runtime | Embedding is `spawn_blocking` (`embed/mod.rs:121-147`). LanceDB/DataFusion queries and `optimize_storage` (`memory/store.rs:1723`, from `daemon/maintenance.rs:136`) run as async tasks — CPU on whatever runtime they are on. The engine's runtime also carries WebRTC (`str0m`), streaming and the UI. | Run memory on **its own tokio runtime** (2–4 worker threads, own blocking pool) inside the process; the engine talks to it through a `Handle`/channel. A slow compaction then cannot stall a voice turn. |
| Memory limits | None found by grep (no RLIMIT or cache limits in `linggen-memory/src` or `src/`; the only thread cap is a 1-worker runtime in `engine/background.rs:510`). Today 279 + 304 MB (plan, measured). One process: a runaway takes both. | Keep the embedder lazy; one embedding at a time (semaphore); cap batch size of `add_batch`; set LanceDB index/metadata cache sizes; the gate records RSS after a dream run. |
| Startup order, lazy model | Today the engine connects to memory late and retries (`server/background.rs:185-230`); core block has a 2 s cap (`engine/prompt/core_block.rs`). `serve::run` opens stores before serving, seeds the change log on first open, loads the embedder in the background. | `memory::open()` before the listener binds (store open is fast; seed is first-run only), embedder stays lazy. The late-server retry loop and pre-warm go. A failed open must not stop the engine: memory routes answer 503, the rest serves. |
| Two processes on one store during takeover | No store lock exists — only the pidfile + "pid alive" check inside `serve::run` (`daemon/serve.rs:80-97`). Old starters remain: the plugin hook runs `ling-mem start`/`restart` every session (`linggen-memory/plugins/linggen/hooks/autostart.sh:99-111`); the plan keeps the old binary on disk for rollback; a stale plugin cache (Codex, OpenClaw) keeps calling it. | (1) An exclusive `flock` on `memory/.store.lock`, held for the process's life by whoever opens the store. (2) Takeover: back up → SIGTERM the pid in `daemon.json` → wait for pid gone and 9528 free → open. (3) Replace `~/.local/bin/ling-mem` with a shim that runs `ling mem "$@"`, refuses `serve/start/stop/restart`, answers `--version` with ≥ 2.3.0 and `status --format json` as running at that version. Then the hook neither starts nor restarts (`autostart.sh:104-111` compares those two values) and `install-bin.sh` never overwrites it (it skips unless the installed binary is strictly older, `install-bin.sh:163-169`) (code). Keep the old binary under another name for rollback. (4) Do **not** write the engine's pid into `daemon.json`: an old `ling-mem stop`/`restart` would SIGTERM the engine. |
| `ling mem …` with / without the engine | CLI probes `daemon.json` → `/api/health`, else **opens the store directly** (`cli/client.rs` header, `try_running_daemon` :51, `try_running_or_start` :76). | Engine up → HTTP to 9527. Engine down → open the store directly **under the same flock**; an engine starting meanwhile waits on the lock (a few seconds) rather than opening a second writer. Never autostart the whole engine from a CLI call. |
| Dependency drift | A fresh resolve broke the trial build (fastembed 5.17 needs candle 0.11; plan). | Commit one workspace `Cargo.lock`; `--locked` in scripts; upgrades only in step 4. |

---

## 4. How a session is bound today

### 4.1 Where each binding lives (code)

| Binding | Stored | Applied | Lifetime |
|---|---|---|---|
| Skill (prompt + tools) | `SessionMeta.skill` (`state_fs/sessions.rs:67-143`); `ChatRequest.skill_name` (`server/chat/types.rs`) picks the store and the bind at creation | `server/chat/handler.rs` ~:740-783: `reload_one` → `prompt_profile.scope_to_app` → `SessionPermissions::load` → `activate_skill(…, SessionBound)` (`engine/skill_activation.rs`) sets `AgentEngine.active_skill`, `Tools.active_skill`, `skill_bound` | **fixed for the session** (`doc/session-spec.md` § What's fixed) |
| Tools | skill `allowed-tools` → `tool_scope::compute_tool_scope`; `SessionPolicy` (`engine/session_policy.rs`: owner/consumer); per-turn `withheld_tools`, `turn_tool_allowlist` | on the cached engine | session, with per-turn narrowing |
| Working directory | `SessionMeta.cwd`; `Tools.root`; engine cache key `(session_id, agent_id)` built once with `project_root` (`AgentManager::get_or_create_session_agent`, `engine/agent/mod.rs:497`) — missions/agents/external pin `meta.cwd` | at engine creation; `cd` changes `cwd` | session |
| Permission | `{session_dir}/permission.json` (`engine/permission/store.rs`) → `AgentEngine.session_permissions`; path-scoped modes; skill grants written at activation | loaded per turn | session |
| Model | `SessionMember.model` per member (`state_fs/sessions.rs:173`); `ChatRequest.model_id`; agent override (`engine/agent/mod.rs:139`) | per member | session, per member |
| Memory scope | `memory_mcp::bound_skill_dir` reads `meta.skill` (`engine/tools/memory_mcp.rs:237-245`) | per call | session |

### 4.2 Yinyue's daily session (code: `server/resident/`)

- Session id `sess-yinyue-YYYY-MM-DD` with `-2`, `-3` segments (`resident/mod.rs`,
  `session_roll.rs`); root `~/.linggen` (`turn.rs::her_root`).
- Two kinds of turn (`turn.rs`): `run_home` on her own thread, and
  `run_at_table` — she is **seated in an app's session** (`chat::members::seat`)
  and runs with that session's `cwd`, `Reach::Sealed`.
- Per-turn knobs already exist: `ready_for_turn` (`turn.rs:376`) sets
  `withheld_tools` from `Reach`, `turn_tool_allowlist` (glance =
  `sense`, `recent_activity`), `turn_language`, `SessionPolicy::owner()`,
  `set_session_id`; `apply_her_model` sets her model each turn.
- So her session is already "one thread, settings chosen per turn" for
  tools/language/model — but **not** for skill, cwd or permission, which
  come from whichever session she is seated in.

### 4.3 What has to change for per-turn binding

1. The skill is **already re-applied each turn**: `run_session_turn`
   (`server/chat/handler.rs:885`) calls `apply_session_bound_skill` (:723),
   which re-reads `meta.skill` when `active_skill` is empty, and every loop
   exit clears `active_skill` (`engine/mod.rs:607,979`, `tool_exec.rs:1545`).
   What is fixed is the *source* (`meta.skill`) and the early return when a
   skill is still set — the precedent for option A.
2. The engine cache key `(session, agent)` assumes one root per session.
3. `permission.json` is one per session directory — grants for 灵境's folder
   would carry into a CFO turn.
4. `memory_mcp::bound_skill_dir` and `prompt_profile.scope_to_app` read the
   session's skill, not the turn's.
5. The prompt prefix (cache) changes when the skill changes — cost per switch.

### 4.4 Options (input for the 2.4 design)

| | Option | For | Against |
|---|---|---|---|
| A | **Turn place overlay**: a `TurnPlace {skill, cwd, grants, tools}` resolved from the page on each request, applied by a generalised `ready_for_turn` and reset after the turn; permission keyed `(session, place)` | one engine, one thread; follows the existing per-turn knobs | every reader of `active_skill`/`meta.skill` must read the turn instead; reset must be total |
| B | **Engine per place**: cache key `(session, agent, place)`; each place has its own engine, prompt prefix and `permission.json` under `sessions/<id>/places/<skill>/`; the thread comes from the shared `messages.jsonl` (already how members share a thread, `doc/shared-session-spec.md`) | least change to the turn core; grants cannot leak between places; prompt cache stays warm per place | more engines in memory; "one person" relies on the shared thread and world state |
| C | Keep per-app sessions, and make "the one session" a view that interleaves them | no engine change | is what the plan removes (per-app daily sessions go away) |

**Recommendation: A, with permissions stored per place** (B's one strong
point). Skill, tools, language and model are already applied at turn start
(4.2, 4.3 item 1), so A extends a path that exists: the turn's page replaces
`meta.skill` as the source, and `permission.json` gains a place key so grants
cannot leak between apps. Fall back to B (engine per place) if the reset at
turn end proves leaky in tests (inferred).

---

## 5. What can be deleted after the merge

| Path | What | Condition |
|---|---|---|
| `src/engine/tools/memory_http.rs` (lines ~314-611: `ling_mem_version`, `resolve_ling_mem`, `fetch_text`, `install_ling_mem`, `try_backup_store`, `prune_backups`, `local_port`, `autostart`, pins and installer URLs) | owning a daemon | always |
| `src/engine/tools/memory_http.rs` (rest) | HTTP client | once callers use the in-process service |
| `src/server/background.rs:87-107` (pre-warm), `watch_for_late_servers` | memory start order | always (loop may stay for user MCP servers) |
| `src/config.rs:301-353` (`ling_mem_url`, `DEFAULT_LING_MEM_PORT`, `$LING_MEM_URL`), `src/engine/types.rs:177-209`, `AgentManager::set_ling_mem_url`, `src/state_fs/client.rs` `ling_mem`/`resolve_ling_mem` | memory address | Q1 = no remote address |
| `src/server/api/ling_mem_client.rs`, `/api/ling-mem/client*` routes, `ui/src/components/settings/LingMemAddress.tsx` | point at another machine | Q1 = no |
| `src/server/api/ling_mem_primary.rs`, `ling_mem_connect.rs` (the `forward`/`relay` halves) | relays | always; the handlers fold into device pages |
| `src/cli/status.rs` `check_ling_mem` (binary/PATH part) | | always |
| `src/telemetry/imp.rs` sibling `ling_mem` detection, `LING_MEM_NO_TELEMETRY` | | always |
| `src/mcp_client/registry.rs:452-579`, `src/mcp_client/client.rs:209-222` | tests gated on a live 9528 | rewrite |
| `linggen-memory/src/update/`, `telemetry/`, `daemon/{lifecycle,pidfile,serve,mod}.rs`, `http/{gate,local,network,health}.rs`, `main.rs` | second updater, telemetry, lifecycle, gate | always |
| `linggen-memory/src/http/ui.rs`, `linggen-memory/static/` | console | Q2 |
| `linggen-memory/src/cli/` lifecycle verbs, `cli/client.rs` daemon discovery by pidfile | | always |
| `linggen-memory/plugins/linggen/hooks/autostart.sh` daemon start + binary bootstrap, `scripts/install-bin.sh` for ling-mem | | after plugins move to 9527 |
| `doc/` ling-mem connection sections (21 docs per plan) | | rewrite |

---

## 6. Oversized files that stand in a split's way

| File | Lines | Why it blocks | Natural cut |
|---|---|---|---|
| `src/config.rs` | 1224 | holds the `config ↔ provider` and `config → state_fs/engine` edges | `config/{types, load, migrate_models}`; only `migrate_models` needs provider |
| `src/engine/tools/mod.rs` | 834 | re-exports leaf helpers others import (`search_exec_find_git_root`, `json_schema`) beside `Tools` | helpers to base; `Tools` stays |
| `src/extensions/skills/mod.rs` | 1466 | parsing (→ skills crate) mixed with loading/registry impl | `parse.rs` vs `loader.rs` |
| `src/extensions/missions/scheduler.rs` | 1281 | runs agents and calls memory; must go up | moves whole; split `stats`/`backup` calls out |
| `src/engine/mod.rs` | 1861 | tests call `extensions` parsers | tests move with the parser |
| `src/server/mcp.rs` (961), `src/server/mcp_agent.rs` (2093) | — | the MCP door merge lands here | tool table vs transport |
| `src/server/chat/handler.rs` | 1836 | session-bound skill activation (~:740-783) is the 2.4 seam | `bind_session_skill` into its own file |
| `src/state_fs/sessions.rs` | 1461 | `SessionMeta` is read by every layer | types vs store |
| `linggen-memory/src/memory/store.rs` (3556), `http/memory.rs` (1864) | — | no split needed for the merge; leave | — |

`src/engine/permission/mod.rs` (1858) is ~98% tests (`mod tests` starts at line 40) — not a blocker.

---

## 7. Open questions for Hanli

1. Keep "use another machine's memory by address" beside primary/replica, or replicas only?
2. Retire memory's own console page (port 9528) in favour of the shared-memory app page?
3. A machine approved with Allow sees all the owner's memories, a phone only its person's — agreed?
4. When Linggen is not running, may `ling mem …` read and write the store directly?
5. Memory sharing over the LAN stays on by default after the merge (9527 open to paired machines)?
6. Rollback to 2.2.1 loses memories written under 2.3 — acceptable, or export them on downgrade?
