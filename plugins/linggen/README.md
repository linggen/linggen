# linggen

> One plugin, one local program, three capabilities: durable cross-session
> **memory**, **agents** that take delegated work, and agent **browser
> control** in your own Chrome — all served by Linggen (`ling`) through one
> MCP server, twelve tools, at one address.

Replaces the `shared-memory` plugin — same memory store, same recall hook,
plus everything else Linggen offers. Never run both.

## Install

```text
claude plugin marketplace add linggen/linggen --sparse .claude-plugin plugins
claude plugin install linggen@linggen
```

Codex:

```text
codex plugin marketplace add linggen/linggen
codex plugin add linggen@linggen
```

No git (a fresh Mac)? `curl -fsSL https://linggen.dev/install-plugin.sh | bash`
installs the release bundle into Claude Code and Codex instead.

That's the whole install. **The plugin's session-start hook installs and
starts Linggen** (`ling`, ~100MB — installed in the background on your first
session with the installer this plugin ships, disclosed in the session context;
log at `~/.linggen/engine-install.log`). Memory lives inside Linggen, so it
comes online when Linggen does. Set `LINGGEN_NO_ENGINE_INSTALL=1` to opt out of
the install and do it yourself: `curl -fsSL https://linggen.dev/install.sh | bash`.

Session start connects **one** MCP server — `linggen`
(`http://127.0.0.1:9527/mcp`: memory, agents, browser) — starts Linggen when
it is installed but not running, and injects your core memory (who you are) into
context, plus — in a project — the directories a memory can be filed under and
the index of standing rules filed for that directory and its parents;
everything else comes back through per-turn recall, scoped to where you work.

**A machine with only the plugin** can use another machine's Linggen and its
memory without installing anything:

```
/linggen:config --linggen 192.168.1.5
```

It asks that machine to connect; click Allow in Linggen there and the token
arrives on its own. That writes `~/.linggen/client.json` — where *this* host
goes looking — probes the address, and mirrors it into Claude Code's
`settings.json` (and Codex's `config.toml`) so the MCP server follows.
Another machine's Linggen shows this host the five memory tools; agents and
browser control stay with the machine they run on. Restart
Claude Code afterwards; MCP URLs resolve at startup. Nothing is installed or
started locally: the hooks speak MCP over HTTP, so there is one store, not two.
`/linggen:config --local` goes back. To keep a synced copy that works offline,
run Linggen on this machine too and use `/linggen:config --primary 192.168.1.5`.

## What you get

- **Memory** (5 tools) — `memory_search` (search, or `op` get / list),
  `memory_add`, `memory_update`, `memory_delete`, and `memory_upkeep` (one
  `op` per step of the scan, dream and solve runbooks) over the one store
  (shared by Claude Code, Codex, Linggen). No account and no model needed. Auto-recall on every prompt via a `UserPromptSubmit` hook; core
  identity injected at session start.
  Linggen recalls for `agent_run` in the delegated session itself. For Claude Code's
  Agent and Task tools, the plugin offers recall to the caller only (`LINGGEN_DELEGATE=0`
  disables it). The delegation hook never changes the call or approves it. Codex's
  built-in `spawn_agent` is not covered (the brief is encrypted).
  Delete is by-id only; rewrites of user-stated facts require the
  user's explicit direction — memory enforces it.
- **Agents** (4 tools) — `agent_run` delegates work to a local Linggen
  agent (in a git worktree you name), `agent_status` (op session / models),
  `agent_permit` answers its permission requests, `agent_end` (op stop /
  close).
- **Browser control** (3 tools) — `browser_open`, `browser_read` and
  `browser_act`, each with an `op`, drive one visible tab in your own
  Chrome via the
  [Linggen browser extension](https://github.com/linggen/linggen/tree/main/browser-extension)
  (Chrome). First action on a new site asks you in the browser;
  payment / credentials / deletes / posting always confirm.

An engine older than this plugin does not publish these tools; the
session-start hook then tells you to update Linggen.

## Requirements

- Linggen (`ling`) on `127.0.0.1:9527` — installed by the session-start hook,
  or another machine's Linggen set with `/linggen:config --linggen`.
- For browser tools: the Linggen browser Chrome extension ([source](https://github.com/linggen/linggen/tree/main/browser-extension)).

## Uninstall

```text
claude plugin uninstall linggen@linggen
```

The memory store under `~/.linggen/memory/` is yours and stays.
