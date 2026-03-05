---
type: spec
guide: |
  Product specification — describe what the system should do and why.
  Keep it brief. Aim to guide design and implementation, not document code.
  Avoid implementation details like function signatures, variable types, or code snippets.
---

# Chat System

Real-time messaging between users and agents, with streaming output, live activity tracking, and task-oriented conversation structure.

## Related docs

- `agentic-loop.md`: how interrupts work in the loop.
- `agents.md`: agent lifecycle events.
- `tools.md`: tool definitions and safety.

## Architecture overview

The backend engine emits events as agents think and act. Events stream to clients via SSE. The UI consumes events, builds message state, and renders an organized conversation.

```
Backend engine → SSE event stream → UI event dispatcher → chat state → renderer
```

Both the Web UI and TUI consume the same event stream.

## Event categories

Events fall into these categories:

| Category | What it conveys |
|:---------|:----------------|
| Token streaming | Model output tokens, including thinking tokens |
| Messages | Chat messages between user and agent |
| Agent status | High-level state transitions: thinking, working, idle |
| Tool lifecycle | Structured content blocks: tool start, progress, completion |
| Delegation | Subagent spawned, subagent finished |
| Context | Token usage, compression, context window stats |
| Plan | Plan created, updated, approved |
| Queue | User message queued while agent is busy |
| Run outcome | Agent turn finished, files changed |

## Agent status lifecycle

Five status values: `idle`, `model_loading`, `thinking`, `calling_tool`, `working`.

Typical progression: `model_loading` → `thinking` → `calling_tool` → `working` → `idle`, but transitions are event-driven and not strictly sequential — the backend emits whichever status reflects current state.

Status events convey high-level transitions. Tool-level detail (which tool, what arguments, success/failure) is conveyed by structured content block events, not status text.

## Message model

Each message carries:

- **Role**: user or agent.
- **Text**: the primary content, rendered as Markdown.
- **Lifecycle state**: whether the message is still being generated (streaming, thinking) or finalized.
- **Activity**: tool calls made during this turn — names, arguments, status, grouped by type.
- **Context stats**: token count, elapsed time, tool call count.
- **Rich content**: subagent delegation tree, structured content blocks, images.

### Message phases

Each agent message progresses through rendering phases:

| Phase | What the user sees |
|:------|:-------------------|
| Thinking | Animated indicator while the model reasons |
| Working | Live activity tree showing tool calls in progress |
| Streaming | Text appearing in real time with a cursor |
| Done | Final message with collapsible activity summary and footer stats |

## Rendering principles

Each chat conversation represents a **task journey** — from the user's question through the agent's reasoning and actions to a final answer. The UI should make this journey clear, organized, and visually rich.
Align WebUI and TUI to Claude Code is the target. 

### 1. Rich, well-ordered messages

Agent messages render as full Markdown: tables, lists, headings, code blocks with syntax highlighting, inline formatting. Content should be concise enough to scan, detailed enough to be useful. No raw JSON, internal status strings, or unformatted tool output in visible messages.

### 2. Live activity tree

While the agent is working, show a tree-structured view of current activity:

- **Primary agent**: tool calls displayed as a grouped, collapsible list (e.g., "Read config.toml", "Edit main.rs") with status indicators (in-progress / done / failed).
- **Subagents** nested under the parent: each delegated agent shown as a branch with its own tool steps, token count, and current status.
- Updates in real time — new tool calls append, completed ones transition from active to done.

### 3. Turn completion summary

When a turn finishes, display a compact summary footer:

- Total tool calls.
- Context tokens used.
- Elapsed time.
- Files changed (if any).

This gives the user an at-a-glance overview of what the agent did.

### 4. Task-oriented conversation structure

Each chat session tells the story of solving a task:

1. **User request** — the question or instruction.
2. **Reasoning** — the agent's thinking and analysis (streaming text, collapsible thinking blocks).
3. **Actions** — tool calls and their results (activity tree, expandable details).
4. **Conclusion** — the final answer, rendered as rich Markdown with clear structure.

The UI should make it easy to follow this narrative: see what the agent explored, understand how it reached its conclusion, and review the final output.

## Rendering pipeline

Each agent message renders as a sequence of inline content blocks, interleaved with text:

```
AgentMessage
  ├─ SubagentTreeView (if delegation occurred)
  ├─ ThinkingIndicator (while model is reasoning)
  ├─ ContentBlockView* (tool calls rendered inline with status indicators)
  ├─ Markdown text (streaming or final)
  └─ TurnSummaryFooter (tool count, tokens, duration — shown when done)
```

Tool calls are rendered inline as `ContentBlockView` items interspersed with text, not grouped into a separate section. Each block shows the tool name, arguments, and status (running / done / failed).

### Special block types

Certain structured payloads render as dedicated widgets:

| Type | Widget | Purpose |
|:-----|:-------|:--------|
| Plan | PlanBlock | Interactive plan approval |
| Finalize | inline in SpecialBlocks | Task completion summary |
| Ask user | AskUserCard | Agent question with options |
| Tool diffs | DiffView (inside ContentBlockView) | File change diffs for Edit/Write |

All other content renders as Markdown.

## Sessions

A session is a single conversation thread scoped to a project. Each session has its own message history, context, and state — completely isolated from other sessions.

### Session lifecycle

- **Creation**: auto-created on first chat when `session_id` is not provided. Format: `sess-{timestamp}-{uuid8}`.
- **Storage**: each session lives at `<project>/.linggen/sessions/<session_id>/` with `session.yaml` (metadata) and `messages.jsonl` (chat history).
- **Persistence**: sessions survive server restarts. The web UI lists all sessions per project.

### Multi-session architecture

Linggen supports multiple active sessions simultaneously. Each client creates its own session:

- **TUI**: starts with no session; the server auto-creates one on the first message. All subsequent messages in that TUI instance reuse the same session.
- **VS Code extension**: same pattern — auto-creates a session per workspace on first chat.
- **Web UI**: can view and switch between all sessions for a project. "New Chat" creates a new session.

### Session isolation

- Different sessions can use the **same agent** (e.g. both use "ling") — the agent engine instance is shared per-project, but chat history is loaded from the session's own message file on each run.
- Different sessions can use the **same model** — model access is controlled by a per-model semaphore (capacity 1). When a model is busy serving one session, other sessions block until the model is free.
- **Context is per-session**: each session has independent message history. Compaction, token counting, and context window management all operate on the session's own messages.

### Model concurrency

Each configured model has a semaphore with capacity 1. When a session's agent calls a model:

1. It acquires the model's semaphore permit.
2. The permit is held for the duration of the streaming response.
3. Other sessions requesting the same model block on `acquire_owned().await`.
4. When the stream completes, the permit is released and the next waiting session proceeds.

This means model busy status is **global** — a model serving project A is unavailable to project B until the request completes. The UI should reflect this: show a loading/busy indicator on all sessions waiting for the same model.

## Message queue

When a user sends a message to a busy agent, it queues. The agent picks it up at the next loop iteration and can react mid-run. This enables the "AI interrupt" pattern — users can redirect, cancel, or query a running agent without waiting.

## Event flow: single agent turn

1. User sends a message.
2. Agent begins thinking — thinking tokens stream to the UI.
3. Agent decides to use a tool — content block events show tool name and arguments.
4. Tool runs — progress events stream output if applicable.
5. Tool completes — content block update marks it done/failed.
6. Agent continues thinking or calls more tools (repeat 3-5).
7. Agent produces a text response — tokens stream as Markdown.
8. Turn completes — summary footer appears with stats.

## API surface

- `POST /api/chat` — send message (queues if busy).
- `POST /api/chat/clear` — clear chat history.
- `GET /api/events` — SSE event stream.
- `GET /api/agent-runs` — list runs for a project/session.
- `GET /api/agent-children` — list child runs (delegation).
- `GET /api/agent-context` — run context and messages.
- `POST /api/agent-cancel` — cancel a run tree.
- `POST /api/ask-user-response` — respond to an AskUser question.
- `POST /api/plan/approve` — approve a pending plan.
- `POST /api/plan/reject` — reject a pending plan.
- `POST /api/plan/edit` — edit a pending plan.
