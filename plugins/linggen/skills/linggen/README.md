# linggen (skill)

**Persistent memory for AI assistants. Local, semantic, typed.**

Linggen's memory, shared by every session, every tool, every project. It is
served by one local program — `ling`, the Linggen engine — and works in Claude
Code, Codex, Linggen, or any agent that can speak MCP or shell out to
a CLI.

## What it does

- **Auto-recall on every prompt.** A `UserPromptSubmit` hook runs a semantic search over your stored facts and injects the top matches as context — no manual tool call required. Relevant preferences and past decisions land in the agent's view automatically.
- **Semantic retrieval.** 1024-dim embeddings via `Qwen3-Embedding-0.6B` (multilingual). Find "berth calibration" by asking about "dock alignment."
- **Typed facts.** `fact`, `preference`, `decision`, `learned`, plus trajectory-level `tried`, `fixed`, `built`. Searches and filters operate on these types.
- **Forgetting is first-class.** Delete by id, forget by filter — refuses empty filters as a guardrail.
- **Local-first storage.** The memory store is on disk in `~/.linggen/memory/` (LanceDB) — no cloud sync. Retrieved facts do enter your agent's prompt context on each turn, so they reach whichever LLM you've configured.
- **One program, one address.** Linggen serves memory, agents, browser control and the person's apps at one MCP door, `http://127.0.0.1:9527/mcp` (thirteen tools; another machine sees the five memory tools). A machine with only the plugin points at another machine's Linggen (`/linggen:config`) and installs nothing.

## Quick start

Install from your agent's marketplace (pick one per host): Claude Code
`claude plugin marketplace add linggen/linggen --sparse .claude-plugin plugins` then
`claude plugin install linggen@linggen`; Codex `codex plugin marketplace add
linggen/linggen` then `codex plugin add linggen@linggen`. Any MCP agent can add `http://127.0.0.1:9527/mcp` as an MCP server (tools only). Linggen (`ling`)
installs on first use.

```bash
# Add a fact
ling mem add "prefers concise replies, no hedging" --type preference --from user

# Semantic search
ling mem search "how do I format logs" --limit 5 --format json

# List by filter
ling mem list --type preference --limit 20

# Forget a specific row
ling mem delete <id>
```

## How each host uses it

| Host | Integration |
|:-----|:------------|
| Claude Code / Codex | The plugin: SKILL.md, the `linggen` MCP server, and hooks — core memory at session start, recall every prompt (`hooks/recall.sh`), scope stamped on every write. |
| Linggen | The engine's own agents reach the same store directly. |
| Standalone | Any MCP client at `/mcp`, or a script on the Linggen machine: `ling mem search "query" --format json` |

## Platforms

- macOS Apple Silicon (M1+) — prebuilt binary
- Linux x86_64 / aarch64 — prebuilt binary

Intel Macs are not supported yet.

## What's stored

- Stored: durable signal you (or the agent on your behalf) add via `memory_add` / `ling mem add`. Indexed in `~/.linggen/memory/memory.lancedb/` (two tables: `semantic` for promoted core/long-term rows, `episodic` for recently-encoded staging).
- Not stored: session transcripts, code you don't explicitly save, anything not added through memory's tools or CLI.

## Links

- **Linggen platform: [linggen.dev](https://linggen.dev)** · [github.com/linggen/linggen](https://github.com/linggen/linggen)
- Plugin + skill source: [github.com/linggen/linggen/tree/main/plugins/linggen](https://github.com/linggen/linggen/tree/main/plugins/linggen)
- Issues: [github.com/linggen/linggen/issues](https://github.com/linggen/linggen/issues)

## License

Apache 2.0, the same as Linggen — see `LICENSE`.
