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

- `skills.md`: dynamic extensions (skill tools).
- `agentic-loop.md`: how tools are dispatched in the loop.
- `agents.md`: per-agent tool access control.

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
| `AskUser` | `questions` | Ask user structured questions mid-execution |

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

## Tool dispatch

Dispatch order in `ToolRegistry.execute()`:

1. **Builtins** — canonical name match.
2. **Skill tools** — HashMap lookup.
3. **Unknown** — error.

**Implementation**: `engine/tool_registry.rs`, `engine/tools.rs`

## Access control

- Per-agent via `spec.tools` in frontmatter. Wildcard `tools: ["*"]` = unrestricted.
- Action gates via policy: `Patch`, `Finalize`, `Delegate`.
- Write-safety mode: checks that file was Read before Write/Edit.
- Tool permission mode: user approval for destructive tools (`Write`, `Edit`, `Bash`, `Patch`).
- Redundancy detection: cache + loop-breaker for repeated calls.

## Tool permission mode

When `tool_permission_mode = "ask"` in `[agent]` config, destructive tool calls require user approval before execution. Default: `"auto"` (no prompting, backward compatible).

**Destructive tools**: `Write`, `Edit`, `Bash`, `Patch`.

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
