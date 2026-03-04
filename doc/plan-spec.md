---
type: spec
guide: |
  Product specification — describe what the system should do and why.
  Keep it brief. Aim to guide design and implementation, not document code.
  Avoid implementation details like function signatures, variable types, or code snippets.
---

# Plan Spec

Redesign of the plan feature, aligned with Claude Code. Separates planning (read-only research) from execution, with user approval gates. Multi-model compatible.

## Related docs

- `agentic-loop.md`: kernel loop, termination.
- `tools.md`: syscall interface, permissions.
- `chat-spec.md`: SSE events, APIs.
- `storage.md`: plan file persistence.

## Design principles

1. **Enforce at infrastructure, not model level.** Remove tools from the API call — don't trust the model to not call them.
2. **Plans are free-form markdown, not structured JSON.** Any model can write text. No JSON parsing for plan creation.
3. **Progress tracking is separate from planning.** `update_plan` is for execution-time task tracking, decoupled from plan mode.
4. **Global plan persistence.** Plans persist to `~/.linggen/plans/{adjective-gerund-noun}.md` with unique generated filenames. Each plan gets its own file.
5. **Self-contained plans.** Plans must include file paths, line numbers, and code snippets so they work as the sole context after clearing.
6. **Unified flow.** No `PlanOrigin` enum — all plans use the same approval flow. Plan mode requires user initiation and approval.

## Changes from previous design

### Dropped

- `PlanOrigin` enum — single unified flow, no `ModelManaged` vs `UserRequested` distinction.
- Per-session plan persistence — now global `~/.linggen/plans/` with unique filenames.
- Plan auto-resume (`load_latest_plan()`) — session resume replaces it.
- `session_plan_dir` field — replaced by `plan_file_path` (full path to the plan file).
- Sidecar `.meta.json` files — simplified to single `.md` file per plan.
- Stale `Planned` → `Executing` auto-promotion — no silent approval bypass.
- Bash in plan mode — blocked entirely for safety.

### Kept

- SSE `PlanUpdate` events.
- `update_plan` action for execution-time progress (separate feature).
- `plan_mode: bool` on engine.
- Status lifecycle: `Planned` → `Approved` → `Executing`.

### Added

- **`ExitPlanMode` tool** — model calls this when plan is ready, triggers approval.
- **Fallback detection** — if model emits `done` in plan mode without calling `ExitPlanMode`, treat as implicit exit.
- **Direct plan editing** — user can edit plan markdown before approving (TUI: `$EDITOR`, Web UI: inline editor).
- **Approval UI** — inline: "Start building" / "Reject" / custom feedback. Web UI: approve endpoint with `clear_context` option.
- **Self-contained plan prompts** — plan-mode prompt emphasizes code snippets and line numbers.
- **Global plan dir** — `plan_file_path` field on engine, unique filename generated per plan via `generate_plan_filename()`.

## Lifecycle

### Entry

1. `/plan <task>` command in Web UI or TUI (user-initiated).
2. Model emits `enter_plan_mode` → engine enters plan mode directly and runs the plan dispatch loop.

### Research phase

Tools available: `Read`, `Glob`, `Grep`, `WebSearch`, `WebFetch`, `AskUser`, `ExitPlanMode`, `UpdatePlan`.

Tools blocked (removed from API call): `Write`, `Edit`, `Bash`, `Task`, `lock_paths`, `unlock_paths`, `Skill`.

Model researches the codebase, then writes a self-contained markdown plan and calls `ExitPlanMode`.

### Approval

Inline approval (via AskUser bridge):
- **Start building** — approves the plan and begins execution.
- **Reject** — plan discarded.
- **Custom text** — user types feedback; model refines plan and re-submits.

Web UI approval (via `/api/plan/approve`):
- Passes `clear_context: bool` to choose whether to clear conversation history.
- Clear context → plan is sole context for execution. Keep context → full conversation retained.

### Execution

Plan text injected into system prompt. Model executes with full tools. Progress tracking via `update_plan` is optional and independent.

### Completion

Not yet implemented. Future: on `done`, mark remaining tracked items as `Skipped`, set plan status → `Completed`, update file, emit SSE event.

## Progress tracking

`update_plan` remains as an independent feature — not coupled to plan mode. Used during execution or spontaneously for complex tasks. Drives the sidebar task list UI.

## Storage

Plan file: `~/.linggen/plans/{adjective-gerund-noun}.md`

Each plan gets a unique filename generated from adjective-gerund-noun word lists (e.g., `bright-running-falcon.md`), collision-checked against the plans directory. Single file per plan, no sidecar metadata. Written on every plan update, overwritten in place.

## Model compatibility

Fallback detection (`done` in plan mode → implicit `ExitPlanMode`) applies to all models unconditionally, so plan mode works regardless of model capability. Models with native tool calling use `ExitPlanMode` directly; models without it rely on the fallback path.

Future: `capability_tier` config field to fine-tune per-model behavior (e.g., disabling `UpdatePlan` for small models).
