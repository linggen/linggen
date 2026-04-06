# Code Review: Linggen vs Specs (2026-04-06)

Comprehensive review of all `doc/*.md` specs against implementation.

## Critical Issues

### C1. Cancellation not checked before/after tool execution
- **Location:** `engine/mod.rs:221`, `tool_exec.rs:923+`
- **Spec:** `agentic-loop.md` — "Checked at loop entry and before/after each tool execution."
- **Problem:** Only checked at iteration boundary. Long-running tools (Bash, WebFetch, Task) delay cancel acknowledgment for their full duration.
- **Fix:** Add cancellation check in `pre_execute_tool` and after `post_execute_tool` returns.
- **Status:** [x] DONE — Added checks in `execute_action_loop` (before each batch), `handle_tool_action` (after tool exec), `handle_parallel_batch` (before each post-exec), and `handle_delegation_batch` (after each join + abort_all).

### ~~C2. Plan-mode tool blocking bypassed when `allowed_tools` is None~~
- **Location:** `engine/dispatch.rs:465`, `engine/types.rs`
- **Spec:** `plan-spec.md` — "Enforce at infrastructure, not model level. Remove tools from the API call."
- **Status:** [x] FALSE POSITIVE — `prepare_loop_messages` (prompt.rs:284-294) correctly converts `None` → `Some(read_only)` in plan mode. The `pre_execute_tool` gate works as intended. No bypass exists in current code.

### ~~C3. `agent_kind` never populated on `AgentRunRecord`~~
- **Location:** `agent_manager/mod.rs:943`
- **Spec:** `agent-spec.md`, `storage-spec.md` — `agent_kind` should be `"main"` or `"subagent"`.
- **Status:** [x] WONTFIX — Run records are in-memory only (see C5). No value in populating a field on ephemeral records. Fix together with C5 if persistence is ever added.

### C4. Mission busy-skip not implemented
- **Location:** `mission_scheduler.rs`
- **Spec:** `mission-spec.md` — "if the mission agent is already running, skip this trigger and log it."
- **Problem:** New trigger blocks on engine lock instead of skipping. No `skipped: true` entry written to `runs.jsonl`.
- **Fix:** Try non-blocking lock before spawning. On failure, record `skipped: true` run entry.
- **Status:** [x] DONE — Added `running: AtomicBool` to `MissionState`. Checked before firing; skipped triggers log `skipped: true` via `record_mission_run`. Flag cleared when dispatch task completes.

### ~~C5. Run records are in-memory only — lost on restart~~
- **Location:** `paths.rs`, `project_store/runs.rs`
- **Spec:** `storage-spec.md` — `~/.linggen/runs/{run_id}.json` should be persisted.
- **Status:** [x] SPEC DRIFT — In-memory is intentional (runs are process-level state, no cleanup needed). Update `storage-spec.md` to remove the `~/.linggen/runs/` section.

### C6. `TurnComplete` emitted with all-None stats
- **Location:** `server/chat_api.rs:1488-1495`
- **Spec:** `chat-spec.md` — turn summary should show "total tool calls, context tokens used, elapsed time."
- **Problem:** `duration_ms: None`, `context_tokens: None` always. UI footer renders nothing.
- **Fix:** Capture start time before dispatch, read context tokens after loop, pass both into `TurnComplete`.
- **Status:** [ ] TODO

---

## Important Issues

### I7. Chat-mode permission block is path-dependent
- **Location:** `engine/permission.rs:836-842`
- **Spec:** `permission-spec.md` — "chat — No tools."
- **Problem:** Tools without a file path (Glob, WebSearch, etc.) may prompt instead of hard-blocking in chat mode.
- **Fix:** Make chat-mode check unconditional for the session's cwd.
- **Status:** [ ] TODO

### I8. `classify_compound_command` splits on bare `&`
- **Location:** `engine/permission.rs:47-64, 657-682`
- **Spec:** `permission-spec.md` — compound commands classified by highest component.
- **Problem:** Splits on individual `&` character, inconsistent with `is_compound_command` which detects `&&`. Latent misparse risk.
- **Fix:** Split on explicit separator patterns (`; | && ||`), not individual `&`.
- **Status:** [ ] TODO

### I9. Read-cache invalidation uses fragile string matching
- **Location:** `engine/tool_exec.rs:639-650`
- **Spec:** `agentic-loop.md` — "Read cache invalidated after Write/Edit to keep observations fresh."
- **Problem:** Different path representations (`./src/main.rs` vs `src/main.rs`) can leave stale cache entries.
- **Fix:** Normalize paths to canonical absolute form before comparison.
- **Status:** [ ] TODO

### I10. Mission `model` override never applied
- **Location:** `mission_scheduler.rs:178-208`
- **Spec:** `mission-spec.md` — missions have optional `model` field.
- **Problem:** `create_mission_session` always writes `model_id: None`. Engine uses default routing chain.
- **Fix:** Set `model_id: mission.model.clone()` in session creation and apply to engine before loop.
- **Status:** [ ] TODO

### I11. Credential env var: dots in model ID not replaced
- **Location:** `credentials.rs:115`
- **Spec:** `models.md` — "hyphens → underscores."
- **Problem:** `gemini-2.0-flash` → `LINGGEN_API_KEY_GEMINI_2.0_FLASH` (dot not portable across shells).
- **Fix:** Also replace `.` with `_` in env var name conversion.
- **Status:** [ ] TODO

### I12. Per-model semaphore has no acquisition timeout
- **Location:** `agent_manager/models.rs:272, 376`
- **Spec:** `chat-spec.md` — model semaphore capacity 1.
- **Problem:** A stalled stream blocks all other users of that model forever. No escape hatch.
- **Fix:** Add a timeout (e.g. 5 min) on `acquire_owned().await`.
- **Status:** [ ] TODO

### I13. WebRTC control channel session messages not implemented
- **Location:** `server/rtc/peer.rs:343-376`
- **Spec:** `webrtc-spec.md` — `session_create`, `session_destroy`, `session_list` should be first-class control channel ops.
- **Problem:** These fall through to "unknown control message type". Works via HTTP proxy workaround.
- **Fix:** Implement the match arms, or update spec to document HTTP proxy approach.
- **Status:** [ ] TODO

### I14. CLI `doctor` aliased to `status`
- **Location:** `main.rs:74, 190-192`
- **Spec:** `cli.md` — `status` = informational, `doctor` = diagnostic checklist with `[OK]/[FAIL]/[INFO]`.
- **Problem:** Both run the same function. Spec defines them as distinct.
- **Fix:** Separate implementations, or update spec to reflect they are unified.
- **Status:** [ ] TODO

### I15. `eval --verbose` flag silently discarded
- **Location:** `main.rs:317`
- **Spec:** `cli.md` — `--verbose` "Print agent messages during execution."
- **Problem:** Flag parsed by clap but never passed to `EvalConfig`.
- **Fix:** Thread `verbose` into `EvalConfig`, or remove the argument.
- **Status:** [ ] TODO

### I16. Turn summary missing "files changed" stat
- **Location:** `ui/src/components/chat/ContentBlockView.tsx:8-33`
- **Spec:** `chat-spec.md` — "Files changed (if any)."
- **Problem:** No `files_changed` field on `TurnComplete` event. Server doesn't track it.
- **Fix:** Add `files_changed: Option<usize>` to `TurnComplete`, populate from engine write/edit count.
- **Status:** [ ] TODO

### I17. `ThinkingIndicator` dead import, not rendered inline
- **Location:** `ui/src/components/chat/AgentMessage.tsx:6, 91`
- **Spec:** `chat-spec.md` — ThinkingIndicator shown inline in message flow.
- **Problem:** Moved to global spinner. Dead import and unused `showThinking` variable remain.
- **Fix:** Either render inline per spec, or clean up dead code and update spec.
- **Status:** [ ] TODO

### I18. `/agent` slash command not in UI autocomplete
- **Location:** `ui/src/components/chat/ChatInput.tsx:221-228`
- **Spec:** `chat-spec.md` — `/agent <name>` switches default agent.
- **Problem:** Missing from `builtinCommands` array and `useChatActions.ts` handlers.
- **Fix:** Add `/agent` to autocomplete and implement handler.
- **Status:** [ ] TODO

---

## Code Style Issues

### S19. `chat_api.rs` is ~1,512 lines with 6+ responsibilities
- **Location:** `server/chat_api.rs`
- **Spec:** `code-style.md` — "split large files into focused modules."
- **Fix:** Extract dispatch functions into `server/chat_dispatch.rs`, plan handlers into `server/plan_api.rs`.
- **Status:** [ ] TODO

### S20. Thinking-channel wiring duplicated across 3 functions
- **Location:** `server/chat_api.rs:682-728, 795-828, 989-1036`
- **Spec:** `code-style.md` — "Repeated logic appears in multiple places" is a refactoring trigger.
- **Fix:** Extract `spawn_thinking_forwarder()` helper.
- **Status:** [ ] TODO

### S21. Dead conditional: both branches identical
- **Location:** `server/mod.rs:911`
- **Code:** `if block_type == "tool_use" { "start" } else { "start" }`
- **Fix:** Replace with `let phase = "start";`.
- **Status:** [ ] TODO

### S22. Project-level `.claude/skills` discovery undocumented in spec
- **Location:** `skills/mod.rs:301-306`
- **Spec:** `skill-spec.md` only lists `.linggen/skills` at project level.
- **Problem:** `.claude/skills` and `.codex/skills` also scanned at project level. Likely intentional for compat but not documented.
- **Fix:** Update spec's discovery priority table.
- **Status:** [ ] TODO

---

## Verified Correct (highlights)

- Agentic loop iteration order (cancel → interrupt → context → model → parse → dispatch → observe)
- Tool registry dispatch order (builtins → skill tools → unknown)
- Plan mode allowed tools include `Task` for delegated research
- Compaction guard during plan execution (`Approved`/`Executing` skip)
- Delegation depth tracking and limiting
- Session ID format `sess-{timestamp}-{uuid8}`
- Session promotion mission→user
- Cron dedup (last fire minute) and daily trigger cap (100/day)
- Permission tier mapping (readonly→Read, standard→Edit, full→Admin)
- Credential resolution priority (TOML → credentials.json → env var)
- Sensitive home paths (`.ssh`, `.gnupg`, `.aws`, `.git/`, `.linggen/`)
- Session path traversal protection (rejects `..`, `/`, `\`)
