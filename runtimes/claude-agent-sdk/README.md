# Claude Agent SDK runtime

The engine launches `runner.ts` with its bundled Bun. The SDK supplies the
official `claude_code` prompt, tools, loop and compaction; agent Markdown
only registers the runtime and optionally appends product instructions.

The dependency version and transitive dependencies are pinned by
`package.json` and `bun.lock`. On first use the engine installs them under
`~/.linggen/runtimes/claude-agent-sdk/<version>/`. Credentials are passed
through the child environment. The isolated Claude config and SDK
transcripts live under `state/`; each Linggen session keeps its own resume
pointer. Linggen's `messages.jsonl` remains the visible conversation log.

The bridge uses JSON lines over stdin/stdout. Every SDK tool passes a
`PreToolUse` hook through Linggen's restrictions and path permissions before
execution. Questions use the existing AskUser bridge. MCP servers inherit
Linggen's configuration, including disabled entries and remote-device
headers; memory calls use the existing host context stamping and write
guards. Stop aborts the SDK and bounds subprocess shutdown.

Run the bridge tests with:

```sh
~/.linggen/bin/bun test runtimes/claude-agent-sdk/runner.test.ts
cargo test --bin ling sdk
```

Live validation uses a disposable project on the normal 9527 engine:
read/edit/test, follow-up resume, denied edit, cancellation, and reloading
the resulting chat. Never use a real project as a destructive fixture.
