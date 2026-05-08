---
type: spec
reader: Coding agent and users
guide: |
  Product specification — describe what the system should do and why.
  Keep it brief. Aim to guide design and implementation, not document code.
  Avoid implementation details like function signatures, variable types, or code snippets.
---

# Permission System

Controls what agents can do, where they can do it, and how access changes as the working folder changes.

## Related docs

- `tool-spec.md`, `session-spec.md`, `agent-spec.md`, `mission-spec.md`, `webrtc-spec.md`, `room-spec.md`.

## Core principle

**A permission is always `(path, mode)`.** The session keeps a list of grants. The effective mode for the current working folder is the most-specific matching grant — or `chat` (no tools) if no grant covers it. If the agent needs a tier higher than the effective mode, interactive sessions prompt; non-interactive runs (missions, proxy consumers) pause or fail.

There is no separate config rule layer, no per-tool ask/deny patterns, and no "reads are free" carveout. Every action — read, write, bash, anything — gates on the same `(path, mode)` lookup.

### What's gated, and what isn't

**The permission system gates the agent. The user is never gated.**

| Actor | Path | Permission check |
|:---|:---|:---|
| Agent's in-process tools (`Bash`, `Read`, `Write`, `Edit`, capability tools) | Through `check_permission` | Yes — path-mode + hardcoded deny floor |
| User-typed `!`-shortcut in chat | `POST /api/bash` | No |
| Skill iframe JS calling backend (e.g. dashboard widgets) | `POST /api/bash` etc. | No |

Rationale: the user is the authority on their own machine. The permission system exists because the model is unpredictable and prompt-injectable — those failure modes don't apply to commands the user typed or to deterministic skill-bundled code the user already approved at skill activation. The hardcoded deny floor is an *agent-only* safety net, not a universal block.

## Modes and tools

| Mode | What's available |
|:-----|:-----------------|
| **chat** | `Skill` only (navigation primitive — always allowed). Any other tool the agent attempts triggers an upgrade prompt offering to switch the folder to the needed tier. The user can Allow once, Switch persistently, or Deny. |
| **read** | `Skill` + `Read`, `Glob`, `Grep`, `WebSearch`, `WebFetch`, `capture_screenshot`, plan tools, `AskUser`, read-class Bash, `Memory_query` |
| **edit** | Everything in read + `Write`, `Edit`, write-class Bash, `Memory_write` |
| **admin** | Everything in edit + admin-class Bash |

`Skill` always bypasses the path-mode check — without it, a chat-mode session could never invoke a skill via natural language. The activated skill then runs through its own permission flow (its `permission:` block, if declared).

Chat mode is *not* a hard wall. It is the lowest tier — every concrete action exceeds it, so the user gets a per-action upgrade prompt. Chat is "default-deny but ask"; pick it when you want explicit approval for every tool the agent uses.

### Bash classification

Bash is the only tool whose tier depends on the command. Each command is classified:

| Class | Examples |
|:------|:---------|
| **read** | `ls`, `cat`, `pwd`, `find`, `grep`, `git status/log/diff/show/branch`, `cargo check`, `npm list` |
| **write** | `mkdir`, `cp`, `mv`, `git add/commit/push`, `npm install/run`, `cargo build/test` |
| **admin** | `kill`, `chmod`, `docker`, `systemctl`, unknown commands |

Compound commands: classified by the highest component. Unknown commands: admin-class.

### Bash path-arg gating

Bash gates on **the paths the command actually touches**, not just the session cwd. Absolute (`/foo`) and tilde (`~/foo`) tokens in the command are extracted; each must be covered by a grant at the command's tier.

- `ls` (no path args) → cwd's tier must cover read.
- `ls /tmp/foo` → `/tmp` must have read; cwd's tier is irrelevant.
- `cat /etc/hosts > /tmp/x` → both `/etc/hosts` (read) and `/tmp/x` (write) checked.
- `bash ~/scripts/foo.sh` → admin tier required on `~/scripts/foo.sh` (or its parent grant).

This prevents `read on /A, cwd /A` from leaking into `bash ls /B` — `/B` would prompt for upgrade. Best-effort extraction: doesn't parse `--flag=/path` forms or quoted paths with spaces. Compound commands (`a; b`, `a && b`) are split and each path checked.

### Per-tool path-gate matrix

What "target path" each tool gates on:

| Tool | Target path | Notes |
|:---|:---|:---|
| `Read`, `Write`, `Edit` | the tool's `path` arg | The actual file being read/written. `Read("/B/x")` is checked against `/B`'s grant, not cwd. |
| `Bash` | each `/`-prefixed and `~/`-prefixed token in `cmd`; falls back to cwd if none | Covered above. |
| `Glob`, `Grep` | cwd | Inherently scoped — the walker descends from cwd; absolute paths in `globs[]` patterns don't actually escape because the walker is cwd-rooted. |
| `Task` | inherits parent's `path_modes` | Subagent runs with parent's grants snapshot at spawn. |
| `WebFetch`, `WebSearch` | cwd | Network ops, but tier-gated on cwd by current design (gate them through chat→read upgrade if you want them to require explicit approval). |
| `AskUser`, plan tools | cwd | Conversational primitives; cwd-tier check applies. |
| `Skill` | none — always allowed | Navigation primitive; the activated skill goes through its own permission flow. |
| `Memory_query`, `Memory_write` | cwd | Routes to a local HTTP daemon; gated by cwd's tier (Read for query, Edit for write per capability registry). |
| `capture_screenshot` | cwd | Network op (URL → image), cwd-tier check applies. |

## Path grants

A grant is `(path, mode)`. The grant covers the path and all children. Each session stores grants in `permission.json`:

```json
{
  "path_modes": [
    { "path": "~/workspace/linggen", "mode": "edit" }
  ]
}
```

That is the entire permission state. No config rules, no session ask-overrides, no denied-call list.

### Effective mode lookup

For the current working folder (or any tool target path):

1. Find the most-specific grant in `path_modes[]` whose `path` is an ancestor of (or equal to) the target. Use that mode.
2. No match → effective mode is **chat**.

That is the entire algorithm. No zones, no `/tmp` special case, no sensitive-path carveouts — every path is treated identically, defaulting to `chat`.

### Session creation

A new user session is given one grant on its starting working folder using the configured default mode (`tool_permission_mode` in `linggen.toml`, default **read**). Anywhere else has no grant — mode is `chat` until upgraded.

### Directory changes

When the agent or user runs `cd`, the engine updates the session cwd, recomputes the effective mode from `path_modes[]`, and emits a mode-change event. The chat header badge updates immediately. The grant list itself does not change.

This contains permission to the granted folder. Edit on project A does not leak to project B; `cd /tmp` lands in `chat` until the user grants `edit` (or higher) there.

### Mode upgrades

When the agent needs a tier higher than the effective mode, interactive sessions prompt:

```
Agent wants to edit src/main.rs

  [Switch this folder to edit]   ← persists (current_cwd, edit) in path_modes
  [Allow once]                   ← one-time approval, no persistence
  [Deny]
```

"Switch this folder to {mode}" is the persistent, always-approve-on-this-path option. After switching, all actions within the new ceiling pass without prompting. This is the only persistence path: **only explicit user approvals, mission frontmatter, and skill frontmatter write to `path_modes[]`**. Allow-once never persists.

### Reads are gated, too

A `Read` of a file outside any grant triggers the same prompt:

```
Agent wants to read /etc/nginx/nginx.conf

  [Switch /etc/nginx to read]
  [Allow once]
  [Deny]
```

Treating reads like writes is required for consumer safety. A remote consumer with a grant on `~/work` cannot reach into `~/.ssh`, because no grant covers it. There is no exception for "reads are cheap."

### Mode in chat widget

The chat header shows the effective mode for the current working folder and updates after `cd`:

```
┌──────────────────────────────────────────┐
│  Session: "Fix auth bug"     [edit ▾]    │
│  ~/workspace/linggen                     │
```

Clicking the badge changes the grant for the current working folder only. It does not change grants for unrelated folders.

## Hardcoded deny floor

A short, curated list of commands is denied at the engine level regardless of mode. Admin mode does not bypass it; the user cannot extend or relax it via config; the floor is baked into the binary.

| Pattern | Why |
|:--------|:----|
| `sudo …` | Privilege escalation — never authorized for an agent |
| `rm -rf /`, `rm -rf /*` | Whole-disk wipe |
| `dd of=/dev/{disk,sd*,nvme*,hd*}` | Direct disk overwrite |
| `mkfs.*` | Filesystem creation on a device |
| `:(){:\|:&};:` | Classic forkbomb |
| `chown -R … /`, `chmod -R … /` | Root-tree ownership/mode bombs |

This is a defense-in-depth floor against the most common foot-guns. Everything else is gated by mode. Users who want stricter control keep the session in `read` mode.

## Permission layers

```
┌─────────────────────────────────────┐
│  1. Hardcoded deny floor            │  Engine-baked, cannot be overridden
├─────────────────────────────────────┤
│  2. Session permission.json         │  path_modes[] — the only persisted state
└─────────────────────────────────────┘
```

No `[permissions]` block in `linggen.toml`. No project-level `permissions.json`. One persistent source per session.

## Check flow

```
 1. Tool in agent's effective set?              NO → blocked
 2. Classify action tier (read / edit / admin)
 3. Resolve target path (cwd if tool has none)
 4. Hardcoded deny floor matches command?       YES → blocked, no override
 5. Most-specific (path, mode) grant for target:
      effective mode tier ≥ action tier         → allowed
      otherwise (or no grant)                   → prompt if interactive,
                                                  pause/fail if unattended
```

Tools without a path target (e.g., `WebSearch`) check the effective mode for the session's current cwd.

## Session creators

Three ways to create a session, each with different initial grants.

### User session

- One grant auto-added on the starting cwd at the configured default mode (`tool_permission_mode`, default `read`).
- User upgrades interactively as needed.
- This is the default.

### Mission session

- Grants come from the mission's `permission:` block (`mode` + `paths`). The mission's `cwd` also receives the mode.
- No prompts during scheduled execution. If a mission needs more than its grants allow, the run records a permission-needed failure/pause.
- Hardcoded deny floor still applies.
- Session promotion (user sends a message): grants are preserved; future permission-needed events become interactive prompts.

### Skill invocation (within a user session)

Skills don't create sessions — they run inside the current user session. Skills that need elevated permissions declare it in frontmatter. **Each path carries its own mode** — there is no top-level default. A skill can ask for `read` on one path and `write` on another in the same block:

```yaml
---
name: pulse
permission:
  paths:
    - { path: ~/.linggen/skills/pulse, mode: write }
    - { path: /tmp,                    mode: read }
  warning: "Pulse writes session JSON inside its own data dir; reads /tmp."
---
```

`mode` per entry is `read`, `edit` (alias `write`), or `admin`. When the user invokes the skill:

```
Skill "pulse" requests grants on: ~/.linggen/skills/pulse (write), /tmp (read)
  ⚠️ Pulse writes session JSON inside its own data dir; reads /tmp.

  [Approve]              ← writes each (path, mode) into session permission.json
  [Run in current mode]  ← skill runs with existing grants, may fail
  [Cancel]
```

Approved grants are added to the current session's `permission.json` and persist for the session. The user can revoke by changing the mode for that path.

Skills without a `permission` section run with whatever grants the session already has.

## Remote access

| Context | Path grants | Permission-needed behavior |
|:--------|:------------|:---------------------------|
| Local browser (owner) | User-controlled | Prompt |
| Remote same-user (owner) | User-controlled | Prompt |
| Remote different-user (guest) | Owner-set | Pause/fail; no guest prompt |
| Proxy consumer (browser) | Owner room config + consumer mode | Block/pause; no consumer prompt |

For proxy consumers, the room config (`allowed_tools`, `allowed_skills`) is the hard ceiling. The consumer's grants live within that ceiling. Consumers cannot upgrade or grant new paths — they have no UI for it, and no prompts ever surface in their session.

## Subagents

- Inherit the parent session's grants (a snapshot at spawn time).
- Can only tighten — cannot upgrade mode (no `AskUser`).
- If a subagent needs more permission, it returns permission-needed to the parent.

## Configuration

```toml
[agent]
tool_permission_mode = "read"    # Default mode for new user sessions
```

That is the only permission-related config. No `[permissions]` block. No deny/ask rule tables. The engine's hardcoded deny floor is fixed and not user-configurable.

## Future

- **Safety classifier**: model-based Bash classification for smarter auto-decisions.
- **OS sandbox**: Seatbelt (macOS) / bubblewrap (Linux) for defense-in-depth.
- **Hooks**: pre/post-tool-use hooks for programmatic permission decisions.
