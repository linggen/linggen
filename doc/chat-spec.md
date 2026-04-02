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

- `session-spec.md`: session/context model, effective tools, prompt assembly.
- `agentic-loop.md`: how interrupts work in the loop.
- `agent-spec.md`: agent lifecycle events.
- `tool-spec.md`: tool definitions and safety.
- `plan-spec.md`: plan mode, approval flow.

## Architecture overview

The backend engine emits events as agents think and act. Events stream to clients via a transport layer. The UI consumes events, builds message state, and renders an organized conversation.

```
Backend engine → events_tx (broadcast) → transport → UI event dispatcher → chat state → renderer
```

### Transport layer

Two transports carry events between the server and clients:

| Transport | Used by | Direction | Protocol |
|:----------|:--------|:----------|:---------|
| **WebRTC** (primary) | Web UI | Bidirectional (data channels) | WHIP signaling + WebRTC data channels |
| **SSE** (legacy) | TUI | Server → client (SSE) + client → server (HTTP POST) | HTTP |

The Web UI uses WebRTC by default (`?transport=webrtc`). Each session gets its own data channel for natural isolation — no broadcast + filter pattern. All API calls are proxied through the WebRTC control channel via a fetch proxy, so the entire Web UI works without direct HTTP access to the server (required for remote mode).

The TUI still uses SSE (`GET /api/events`) and HTTP POST for sending messages. The SSE endpoint and broadcast channel remain active to support TUI clients.

See `webrtc-spec.md` for the full WebRTC design.

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
| Widget resolved | Interactive widget dismissed (permission prompt, etc.) |

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

### Session-bound skills

A session can optionally bind to a skill at creation time via the `skill` field in `POST /api/sessions`. When bound:

- The skill is active for **every message** in the session (not just one invocation).
- `effective_tools = intersection(agent.tools, skill.allowed-tools)`.
- The skill body is injected into the system prompt on every turn.
- Tool-related prompt sections are skipped when effective tools is empty.

This enables interactive app skills (e.g., a game UI that creates a session bound to `game-table`). The session stays skill-scoped for its entire lifetime. No "recovery" mechanism — other sessions are unaffected.

### Session isolation

- Different sessions can use the **same agent** (e.g. both use "ling") — the agent engine instance is shared per-project, but chat history is loaded from the session's own message file on each run.
- Different sessions can have **different bound skills** — one session can be a game, another can be coding, both using ling.
- Different sessions can use the **same model** — model access is controlled by a per-model semaphore (capacity 1). When a model is busy serving one session, other sessions block until the model is free.
- **Context is per-session**: each session has independent message history. Compaction, token counting, and context window management all operate on the session's own messages.

### Model concurrency

Each configured model has a semaphore with capacity 1. When a session's agent calls a model:

1. It acquires the model's semaphore permit.
2. The permit is held for the duration of the streaming response.
3. Other sessions requesting the same model block on `acquire_owned().await`.
4. When the stream completes, the permit is released and the next waiting session proceeds.

This means model busy status is **global** — a model serving project A is unavailable to project B until the request completes. The UI should reflect this: show a loading/busy indicator on all sessions waiting for the same model.

## Widget sync across clients

Interactive widgets (permission prompts, plan approval) can be displayed on multiple clients simultaneously (localhost + remote). When a user responds on one client, all other clients must dismiss the widget.

Pattern: the server emits a `WidgetResolved { widget_id }` event after processing the user's response. All connected clients receive this event and dismiss the matching widget. The `widget_id` corresponds to the original widget identifier (e.g., `question_id` for AskUser prompts).

Plan approval/rejection is already synced via PlanUpdate status changes — no separate WidgetResolved needed.

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

The server exposes REST + WebRTC + SSE endpoints for chat, sessions, agents, models, skills, missions, and workspace operations.

### Web UI (WebRTC transport)

All Web UI communication goes through WebRTC data channels:

- **Chat messages, plan actions, AskUser responses**: sent via the control channel RPC (request/response pattern).
- **Events (tokens, activity, content blocks)**: received on per-session data channels (`sess-{id}`).
- **Other API calls** (config, status, files, sessions, etc.): transparently proxied through the control channel's `http_request` message type via a global fetch proxy.
- **WHIP signaling**: `POST /api/rtc/whip` — the only direct HTTP call, used to establish the WebRTC connection.

### TUI (SSE transport, legacy)

- **SSE**: `GET /api/events?session_id=...` streams real-time events. Each event carries an optional `session_id`; the TUI filters by active session.
- **HTTP POST**: `POST /api/chat` and other REST endpoints for sending messages and commands.

### REST endpoints

Both transports ultimately hit the same REST API handlers on the server. Key patterns:

- **Chat**: send messages, clear history, force compaction.
- **Agent runs**: list, inspect context, cancel run trees.
- **Interactive**: respond to AskUser questions, approve/reject/edit plans.
- **CRUD**: sessions, projects, missions, skills, models, credentials.

Endpoints evolve frequently — see `server/mod.rs` for the authoritative route list.

## Slash commands

Available in both TUI and Web UI. Handled client-side (not sent to the agent).

| Command | Description |
|:--------|:------------|
| `/help` | Show available commands |
| `/clear` | Clear chat context |
| `/compact [focus]` | Force context compaction (summarize old messages). Optional focus guides the summarizer. |
| `/status` | Show project status, models, token usage |
| `/model [id]` | List models or switch default model |
| `/agent <name>` | Switch default agent |
| `/plan <task>` | Ask agent to create a plan (read-only) |
| `/plan approve` | Approve and execute the pending plan |
| `/plan reject` | Reject the pending plan |
| `/image <path>` | Attach an image file |
| `/paste` | Paste image from clipboard (TUI) |
| `@path` | Mention a file (autocomplete on `@`) |
| `@agent message` | Send to a specific agent |
