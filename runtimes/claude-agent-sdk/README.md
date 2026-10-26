# Claude Agent SDK runtime

New Claude session starts one Claude Agent independently of native Linggen
agents. No agent Markdown is required. `server/claude/` owns the hosting
adapter; no native AgentEngine is created for these turns.

The SDK owns its official `claude_code` prompt, tools, loop, project settings,
context, compaction and permissions. Linggen owns the UI and process hosting.
`PreToolUse` observes tools without allowing or denying them. `canUseTool`
forwards SDK approval requests and AskUserQuestion to shared widgets, then
returns the person's answer to the SDK. Native path grants are never consulted.
The picker exposes SDK modes `default`, `acceptEdits`, `auto`, `plan` and
`dontAsk`. Live changes wait for the SDK's acknowledgement; an unavailable
mode returns the SDK error. Session approvals save the SDK's suggested rules
in `claude-permissions.json` and rehydrate them through SDK settings on resume,
without changing project or global permissions. A session approval never
implicitly changes permission mode.

The slash menu comes from `supportedCommands()` and native SDK command-list
updates. Commands go to the SDK, including `/clear` and `/compact`. `!` is a
shared host input shortcut for direct shell execution. The only extra SDK
tool is `mcp__linggen-display__DisplayImage`, which reuses Linggen's image
store, references, thumbnails and lightbox. SDK Read image results use that
same store. Tool execution and approvals still belong to the SDK.

Memory matches a native owner session: the same session-start block (core,
scope candidates, index) is appended to the `claude_code` preset prompt, and
each turn's recall and capture nudge reach the SDK as `UserPromptSubmit`
context, with the same recall row in the chat. Slash commands get no recall.
The SDK's own config dir is Linggen's, so plugin hooks from `~/.claude` do
not run here; this host supplies memory instead.

Linggen apps only use native Linggen sessions. They cannot create, bind or
run a Claude session, or delegate to one through `StartClaudeSession`.
An ordinary Ling chat can still start a separate Claude session.

Bun and pinned dependencies run the bridge. Credentials stay in the child
environment; official product attribution is preserved. SDK state lives under
`~/.linggen/runtimes/claude-agent-sdk/state/`. Each Claude session keeps a
`claude.json` resume pointer. Its `messages.jsonl` is a display record and is
never replayed into the SDK. SDK `/clear` switches the pointer to the new SDK
conversation and clears the displayed transcript and session grants. The
host clear action cancels the turn and removes the pointer and grants.

Run bridge tests with `bun test runtimes/claude-agent-sdk/runner.test.ts` and
host tests with `cargo nextest run --bin ling -E 'test(server::claude)'`.
Live validation uses a disposable folder on the normal 9527 engine: creation,
read/edit/test, follow-up and restart resume, denial, AskUser, plan and stop.
