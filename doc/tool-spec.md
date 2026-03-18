---
type: spec
guide: |
  Product specification — describe what the system should do and why.
  Keep it brief. Aim to guide design and implementation, not document code.
  Avoid implementation details like function signatures, variable types, or code snippets.
---

# Tools

Syscall interface: built-in tools, safety rules, and the two-tier model.

## Related docs

- `session-spec.md`: effective tools, capability model.
- `skill-spec.md`: dynamic extensions (skill tools).
- `agentic-loop.md`: how tools are dispatched in the loop.
- `agent-spec.md`: agent tool declarations.

## Two-tier model

| Tier | What | Why built-in |
|:-----|:-----|:-------------|
| **Built-in tools** | Core coding ops | Need safety enforcement, internal state, or performance |
| **Skills** | Everything else | Extensible, replaceable, no code changes |

Built-in tools are the kernel API. Skills are userspace.

## Built-in tools

| Tool | Args | Purpose |
|:-----|:-----|:--------|
| `Glob` | `globs?, max_results?` | File pattern matching |
| `Read` | `path, max_bytes?, line_range?` | Read file contents |
| `Grep` | `query, globs?, max_results?` | Content search (ripgrep) |
| `Write` | `path, content` | Create/overwrite file |
| `Edit` | `path, old_string, new_string, replace_all?` | String replacement in file |
| `Bash` | `cmd, timeout_ms?` | Shell command execution |
| `capture_screenshot` | `url, delay_ms?` | Web page screenshot |
| `lock_paths` | `globs, ttl_ms?` | Acquire file locks (multi-agent) |
| `unlock_paths` | `tokens` | Release file locks |
| `Task` | `target_agent_id, task` | Spawn subagent |
| `WebSearch` | `query, max_results?` | Web search (DuckDuckGo) |
| `WebFetch` | `url, max_bytes?` | Fetch URL content as text |
| `Skill` | `skill, args?` | Invoke a skill by name |
| `RunApp` | `skill, args?` | Launch an app-enabled skill |
| `AskUser` | `questions` | Ask user structured questions mid-execution |
| `EnterPlanMode` | `reason?` | Enter plan mode (research-only) |
| `ExitPlanMode` | `plan_text` | Exit plan mode with completed plan |
| `UpdatePlan` | `plan_text?, items?` | Update plan progress during execution |

**Aliases**: `Read`/`Write`/`Edit` accept `file`/`filepath` for `path`. `Edit` accepts `old`/`search`/`from` for `old_string`, `new`/`replace`/`to` for `new_string`.

Tool names follow Claude Code convention (capitalized).

## AskUser

Lets the agent pause mid-loop to ask the user structured questions. Aligned with Claude Code's `AskUserQuestion`.

- **1-4 questions** per call, each with 2-6 selectable options.
- User can always type custom text ("Other").
- Blocks the agent loop until user responds (5 min timeout).
- Not available in delegated sub-agents — returns error.
- Not available in CLI mode.
- UI renders an inline card with option buttons in the chat stream.
- Cancelling the agent run unblocks the tool gracefully.

**Flow**: tool emits `AskUser` SSE event → UI renders card → user submits via `POST /api/ask-user-response` → oneshot channel delivers answer → loop continues.

**Implementation**: `engine/tools.rs` (execution), `server/chat_api.rs` (response endpoint), `ui/src/components/AskUserCard.tsx` (UI).

## RunApp

Launches an app-enabled skill. The skill must have an `app` section in its frontmatter with a `launcher` type. See `skill-spec.md` → App skills.

- **`web`**: serves the skill directory as static files at `/apps/{skill-name}/`, emits `AppLaunched` SSE event, UI opens an iframe panel.
- **`bash`**: executes the entry script in the skill directory, returns stdout/stderr.
- **`url`**: emits `AppLaunched` SSE event with the external URL, UI opens in panel or new tab.

**Direct invocation**: when a user invokes an app skill via `/skill-name`, the server short-circuits the model — executes the app directly without entering the agent loop.

**Model invocation**: any agent can call `RunApp` to launch an app during a conversation (e.g., "let me open the dashboard for you").

**SSE event**: `AppLaunched { skill, launcher, url, title, width?, height? }` — tells the UI to open the app panel.

**Implementation**: `engine/tools/delegation.rs` (`run_app()`), `server/chat_api.rs` (direct dispatch), `server/mod.rs` (static serving at `/apps/`), `ui/src/components/AppPanel.tsx` (UI).

## Tool dispatch

Dispatch order in `ToolRegistry.execute()`:

1. **Builtins** — canonical name match.
2. **Skill tools** — HashMap lookup.
3. **Unknown** — error.

**Implementation**: `engine/tool_registry.rs`, `engine/tools.rs`

## Access control

- Per-agent via `spec.tools` in frontmatter. Wildcard `tools: ["*"]` = unrestricted.
- Capabilities determined by effective tool set (see `session-spec.md`): Write/Edit = can patch, Task = can delegate.
- Write-safety mode: checks that file was Read before Write/Edit.
- Tool permission mode: user approval for destructive tools (`Write`, `Edit`, `Bash`).
- Redundancy detection: cache + loop-breaker for repeated calls.

## Tool permission mode

Three modes:

- **`ask`** (default) — destructive tool calls require user approval before execution.
- **`accept_edits`** — auto-approve `Write`/`Edit` but still prompt for `Bash`.
- **`auto`** — no prompting, all tools auto-approved.

**Destructive tools**: `Write`, `Edit`, `Bash`.

**Approval options**:
- **Allow once** — proceed this one time only.
- **Allow all {tool} for this session** — session-scoped, in-memory.
- **Allow all {tool} for this project** — persisted to `{workspace}/.linggen/permissions.json`.
- **Cancel** — deny the tool call; agent sees a denial message.

**Flow**: permission gate in `handle_tool_action()` → check `PermissionStore` → if not allowed, emit `AskUser` SSE event (header="Permission") → await user response → proceed or deny.

Reuses the AskUser bridge — no new endpoints. Web UI renders `ToolPermissionCard`, TUI renders `InteractivePrompt`.

**Implementation**: `engine/permission.rs` (store, helpers), `engine/mod.rs` (gate, `ask_permission()`), `ui/src/components/ToolPermissionCard.tsx` (UI).

## File safety

- All paths sanitized to workspace root.
- Parent traversal (`..`) rejected.
- Absolute paths outside workspace rejected.
- File listing/search use ignore-aware walking.

## Bash safety

- Commands executed via `sh -c` in workspace root.
- Timeout enforced (default 30s).
- Output captured (stdout + stderr).
- Workspace-scoped: working directory set to workspace root.
