---
type: spec
guide: |
  Product specification — describe what the system should do and why.
  Keep it brief. Aim to guide design and implementation, not document code.
  Avoid implementation details like function signatures, variable types, or code snippets.
---

# Agentic Loop

The kernel of linggen. Every agent run — chat, idle prompt, delegation — is one loop instance.

## Related docs

- `product-spec.md`: vision, OS analogy.
- `agent-spec.md`: process management, delegation.
- `tool-spec.md`: syscall interface.
- `chat-spec.md`: IPC, message queue.

## Loop iteration

```
for iter in 0..max_iters {
    1. Check cancellation          → abort if cancelled
    2. Check user message queue    → inject into context if present
    3. Build context               → system prompt + history + observations
    4. Call model                  → streaming response
    5. Parse actions               → tool calls, delegation, done
    6. Execute actions             → dispatch tools, spawn subagents
    7. Feed observations           → results back into history
}
```

**Termination**: Done (plain text answer), EnterPlanMode, cancellation, or `max_iters` reached.

Same loop for all execution types: user chat, idle prompt, delegation.

**Implementation**: `engine/mod.rs` → `run_agent_loop()`

## User message queue (interrupt)

While the loop runs, users can send messages. These are queued per-agent.

At the **top of each iteration**, the engine checks the queue. If present, messages are injected into context. The model then decides:

| User says | Model decides |
|:----------|:-------------|
| "cancel" | Stop loop, report progress |
| "wait" | Pause, ask what's up |
| "change to X" | Adapt plan, continue |
| "progress?" | Report status, continue |

This is **cooperative interruption** — the loop yields at each iteration boundary. The model handles all interrupt logic, no hardcoded signal handlers.

Messages are queued per-agent and checked at each iteration boundary via an interrupt channel.

## Cancellation (signals)

- Checked at loop entry and before/after each tool execution.
- `cancel_run_tree()` cascades to all descendants.
- Run status persisted (`running` → `cancelled`).
- User can also cancel via message queue (model decides to stop gracefully).

**Implementation**: `agent_manager/mod.rs` → `is_run_cancelled()`, `cancel_run_tree()`

## Context management

- Context is built from system prompt + conversation history + tool observations.
- When context approaches token limit, automatic compaction summarizes older messages using model-based summarization (falls back to deterministic extraction on failure).
- `/compact [focus]` command forces compaction regardless of budget. Optional focus parameter guides the summarizer to emphasize specific topics.
- High/Critical importance messages survive compaction; Low/Normal messages are summarized.
- Tool results are trimmed to fit within limits.
- Read cache invalidated after Write/Edit to keep observations fresh.

## Guardrails

The loop includes gates and streak detection:

- **Permission gate** — destructive tools (`Write`, `Edit`, `Bash`, `Patch`) require user approval when `tool_permission_mode = "ask"`. See `tool-spec.md`.
- Empty search results → nudge to broaden query.
- Redundant tool calls → nudge to try different approach.
- Invalid JSON parsing → retry hint.
- Repetition detection → loop breaker prompt.
