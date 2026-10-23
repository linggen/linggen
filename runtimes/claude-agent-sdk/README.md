# Claude Agent SDK runtime

New Claude session starts one Claude Agent independently of native Linggen
agents. No agent Markdown is required. `server/claude/` owns the hosting
adapter; no native AgentEngine is created for these turns.

The SDK owns its official `claude_code` prompt, tools, loop, project settings,
context, compaction and permissions. Linggen owns the UI and process hosting.
`PreToolUse` observes tools without allowing or denying them. `canUseTool`
forwards SDK approval requests and AskUserQuestion to shared widgets, then
returns the person's answer to the SDK. Native path grants are never consulted.

Bun and pinned dependencies run the bridge. Credentials stay in the child
environment; official product attribution is preserved. SDK state lives under
`~/.linggen/runtimes/claude-agent-sdk/state/`. Each Claude session keeps a
`claude.json` resume pointer. Its `messages.jsonl` is a display record and is
never replayed into the SDK. Clearing the chat resets the pointer.

Run bridge tests with `bun test runtimes/claude-agent-sdk/runner.test.ts` and
host tests with `cargo nextest run --bin ling -E 'test(server::claude)'`.
Live validation uses a disposable folder on the normal 9527 engine: creation,
read/edit/test, follow-up and restart resume, denial, AskUser, plan and stop.
