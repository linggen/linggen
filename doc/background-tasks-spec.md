---
type: spec
reader: Coding agent and users
guide: |
  Product specification — describe what the system should do and why.
  Keep it brief. Aim to guide design and implementation, not document code.
  Avoid implementation details like function signatures, variable types, or code snippets.
---

# Background tasks

Work an agent starts and does not wait for: a long shell command, a
subagent, a watch on a log. It runs beside the chat, never in front of it.
The same in a native Linggen session and in a Claude session.

## Related docs

- `chat-spec.md`: message queue, Steer, the subagent pane.
- `session-spec.md`: Claude sessions, the SDK runner.
- `agent-spec.md`: delegation (`Task`).
- `tool-spec.md`: `Bash`.

## Rules

1. **Never blocks the chat.** A turn that started background work ends when
   its reply is written. The person keeps talking; their messages start
   turns as usual. The agent is not "busy" because a task is running.
2. **Ends as a turn.** When a task finishes, fails or is stopped, the agent
   is told in a new turn of its own (who started it, how it ended, a short
   summary, where the full output is). No person's message is needed. Only
   the agent that started it is told: a subagent's own task never starts a
   turn of the session, and its end is never said as the main agent's.
3. **Visible and in the person's hands.** Every background task of the
   session is listed while it runs and for a while after: what it is, how
   long it has run, its latest output or activity, how it ended. Any
   running task can be stopped from the list. Stopping one never stops the
   turn or the other tasks; Stop on the turn never stops background tasks.
4. **One list, one surface.** Both runtimes feed one session task list. The
   subagent pane shows it together with the session's subagents, each item
   once (a background subagent is one row, not a task and a subagent):
   running on top, ended folded below. A row opens its detail in the same
   pane — a subagent's rich chat view, or a shell or watch task's command,
   status and output tail — and back returns to the list. A subagent's row
   and detail show its context window, as the chat header shows the main
   one's. A long task
   does not hold the pane open; when the pane is closed, the chat header
   shows how many run and opens the pane, at the list.
5. **What ends them.** Finishing, the person's stop, the agent's own stop,
   Clear or delete of the session, an engine restart. A task the engine
   restart ended is listed as interrupted, not as failed. Clear and delete
   stop a native session's tasks before the session is cleared or removed,
   and their ends are told to no agent; a Claude session's go with its
   runner.

## Kinds

| Kind | Native session | Claude session |
|:-----|:---------------|:---------------|
| Shell | `Bash` with `run_in_background` | SDK background shell |
| Subagent | `Task` with `run_in_background` | SDK background subagent |
| Watch | — | SDK Monitor / MCP task |

A native agent reads a task's output and stops it with tools of its own
(`TaskOutput`, `TaskStop`); a Claude agent uses the SDK's.

## Claude sessions: the runner lives while work does

The SDK keeps background tasks inside its process. So the runner process
stays up past a turn's result while the session has queued messages,
background tasks or an active goal, and exits only when idle for a while
(and on Clear, delete, an engine restart). Each turn — a person's message,
a task's notification — is its own run in the session, with its own run
record and busy state; the runner is not. A person's message reaches the
live runner directly. Per-turn context (memory recall) rides each message.
A change of model, MCP servers or folder trust ends the runner at its next
idle, and the next turn starts a fresh one on the new settings.

## Engine restart

An agent restarts the engine it runs in on a new build with `ling restart`
(`cli.md`). Every session whose turn the restart interrupts continues
afterwards, not only the one that asked:

1. **Wait first.** The engine waits up to two minutes for other sessions'
   turns to finish, so ideally only the caller is interrupted.
2. **Remember who.** It records each session with a running turn (missions
   excepted: their catch-up covers them), its running background tasks and
   the messages queued in any session, then a detached helper swaps the
   build, restarts, and rolls back if the new engine is not healthy.
3. **Say what happened.** Once the helper has said how it went, the engine
   gives each recorded session a turn: the old and new version, healthy or
   rolled back, that its last step may not have finished and it should check
   the state before redoing anything that changes things, and which
   background tasks the restart ended. A Claude session continues the same
   SDK conversation. Idle sessions are not touched. Each plan runs once.
4. **Keep the queue.** Messages the person had queued in a session (their
   text and pictures) are kept in the plan and sent again, in their order,
   behind that session's continue turn (if it has one), back in the queue
   like typed ones. A native one waits for that turn rather than cutting it
   short.

## Native sessions

A background shell is a child process of the engine, owned by the session.
Its output goes to a file in the session's directory; the list shows the
tail. A background subagent runs on its own engine like any delegation,
but the parent's tool call returns at once with the task's id.
