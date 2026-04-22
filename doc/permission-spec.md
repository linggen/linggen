---
type: spec
guide: |
  Product specification — describe what the system should do and why.
  Keep it brief. Aim to guide design and implementation, not document code.
  Avoid implementation details like function signatures, variable types, or code snippets.
---

# Permission System

Controls what agents can do, who can grant access, and how trust flows across local, remote, and proxy contexts.

## Related docs

- `tool-spec.md`, `session-spec.md`, `agent-spec.md`, `mission-spec.md`, `webrtc-spec.md`, `proxy-spec.md`.

## Core principle

**Session is the single source of truth for permissions. Every permission must be preset or approved by the user.** No tool permissions in agent specs, skill frontmatter defines requests not grants. The agent cannot gain capabilities without explicit human consent.

## Design principles

1. **Nothing auto-allowed.** All tool access requires explicit user consent — preset via config or approved at runtime.
2. **Deny wins.** A deny rule blocks regardless of allows elsewhere.
3. **Inner layer can only tighten.** Subagents restrict, never expand.
4. **Prompt only when someone can answer.** Missions and proxy sessions use pre-configured permissions only.
5. **Classify the action, not the tool.** `Bash("ls")` is a read; `Bash("rm -rf /")` is admin. Permission reflects what the command *does*.
6. **Session is the permission boundary.** Each session has its own permission state. No project-level persistence.
7. **Align with OS filesystem ownership.** Home directory is user space — mode switching allowed. System directories require per-action approval — no persistent write grants. This mirrors Unix (`~/` vs `/etc`) and Windows (`%USERPROFILE%` vs `C:\Windows`).

## Modes and tools

Session mode controls which tools are available. No tool permissions in agent specs, skill frontmatter, or elsewhere — **session is the single source of truth**.

| Mode | What's available |
|:-----|:-----------------|
| **chat** | No tools |
| **read** | `Read`, `Glob`, `Grep`, `WebSearch`, `capture_screenshot`, plan tools, `AskUser`, read-class Bash, `Memory.{search,get,list}` |
| **edit** | Everything in read + `Write`, `Edit`, write-class Bash, `Memory.{add,update,delete}` |
| **admin** | Everything, including `Memory.forget` (bulk-delete) |

### Bash classification

Bash is the only tool whose mode depends on the command. Each command is classified:

| Class | Examples |
|:------|:---------|
| **read** | `ls`, `cat`, `pwd`, `find`, `grep`, `git status/log/diff/show/branch`, `cargo check`, `npm list` |
| **write** | `mkdir`, `cp`, `mv`, `git add/commit/push`, `npm install/run`, `cargo build/test` |
| **admin** | `rm`, `sudo`, `kill`, `chmod`, `docker`, `systemctl`, unknown commands |

Compound commands: classified by highest component. Unknown: admin-class.

## Path zones

The filesystem is divided into zones, aligned with OS ownership:

| Zone | Unix/macOS | Windows | Mode switch | Writes |
|:-----|:-----------|:--------|:-----------|:-------|
| **Home** | `~/` | `%USERPROFILE%` | Yes | Within mode ceiling |
| **Temp** | `/tmp`, `/var/tmp` | `%TEMP%` | Yes | Within mode ceiling |
| **System** | Everything else | `C:\Windows`, `C:\Program Files` | **No** | **Per-action only** |

**Sensitive home paths** (`~/.ssh`, `~/.gnupg`, `~/.aws`, `.git/`, `.linggen/`) behave like system zone — per-action approval for writes, no mode switch.

## Permission modes

Four modes. Each defines a ceiling — the max tier the agent can access on a given path.

| Mode | Ceiling | What the agent can do |
|:-----|:--------|:---------------------|
| **chat** | chat | Converse only. No tools. |
| **read** | read | Read, search, inspect. |
| **edit** | edit | Read + write/edit files. |
| **admin** | admin | Everything including Bash, web, system ops. |

**Default: read**, scoped to the session's starting directory.

### Modes are path-scoped

A mode grant is always `(path, mode)`. The grant covers the path and all children. Each session stores a list of grants in `permission.json`:

```json
{
  "path_modes": [
    { "path": "~/workspace/linggen", "mode": "edit" },
    { "path": "/etc/nginx", "mode": "read" }
  ]
}
```

Most specific matching path wins.

### Mode upgrades

When the agent exceeds the current path's ceiling (home/temp zone), the prompt for that specific tool call includes a mode switch option:

```
Agent wants to edit src/main.rs

  [Switch to edit mode]     ← grants edit on ~/workspace/linggen/**
  [Allow once]              ← one-time, mode stays the same
  [Deny]
```

"Switch to edit mode" persists in `permission.json`. Future edits and write-class Bash within that tree pass without prompting. This applies to all tool types — Edit, Write, and write-class Bash all show the mode switch option when they exceed the ceiling.

### Directory changes

When the working directory changes (`! cd ~/other-project`):

- If the new path has an existing grant → that mode applies.
- If no grant covers it → effective mode resets to **read**.

Edit mode on project A does not leak to project B.

### Reading outside granted paths

If the agent reads a file not covered by any grant:

```
Agent wants to read /etc/nginx/nginx.conf

  [Allow read on /etc/nginx]    ← grants read for the session
  [Allow once]
  [Deny]
```

### System zone writes

No mode switch offered. Always per-action:

```
Agent wants to edit /etc/hosts

  [Allow once]
  [Deny]
```

### Mode in chat widget

Current effective mode shown in chat header, updates on directory change:

```
┌──────────────────────────────────────────┐
│  Session: "Fix auth bug"     [edit ▾]    │
│  ~/workspace/linggen                     │
```

Clicking the badge opens a dropdown to switch modes for the current path.

### Locked flag

Not a mode — a flag. When locked, prompts are skipped; actions that would need prompting are blocked instead. Used for missions, CI, remote-different-user sessions.

## Permission layers

```
┌─────────────────────────────────────┐
│  1. Config (linggen.toml)           │  Deny/ask rules, default mode
├─────────────────────────────────────┤
│  2. Session (permission.json)       │  Path modes, policy, session allows, denied sigs
└─────────────────────────────────────┘
```

Config sets guardrails. Session holds runtime state. That's it.

## Session policy

Mode sets the *capability ceiling* on each path. **Policy** decides what happens when an action exceeds the ceiling or hits an ask-rule. The two concepts are orthogonal and compose: mode = what the agent is allowed to do, policy = behavior when it wants more.

Two independent levers:

| Lever | When it applies | Choices |
|:------|:----------------|:--------|
| **on_exceed** | Action exceeds effective path-mode grant | `ask` / `allow` / `deny` |
| **on_ask_rule** | Action matches an `ask:` rule | `ask` / `allow` / `deny` |

Deny rules always deny — policy cannot override the safety floor.

Named presets:

| Preset | `on_exceed` | `on_ask_rule` | When to use |
|:-------|:-----------:|:-------------:|:------------|
| **interactive** | ask | ask | Default for user-facing sessions |
| **strict** | deny | deny | Autonomous runs where safety matters more than coverage |
| **trusted** | allow | deny | Autonomous runs you trust — legacy locked-session behavior |
| **sandbox** | allow | allow | Containerized/Docker runs where the OS is the guardrail |

Policy applies to the *whole session*. It's set by:

- Interactive user sessions → `interactive` (default)
- Consumer (proxy-room) sessions → `trusted` (no prompts, still denies `ask:` rules like `git push`)
- Mission sessions → declared in the mission's `policy:` field, defaults to `trusted` for backward-compat. Strict is opt-in for safer runs.

A policy where either lever is not `ask` is *locked* — the agent never prompts the user in that session.

## Permission rules (deny / ask)

Two types in config. Deny takes priority over ask.

| Type | Effect |
|:-----|:-------|
| **deny** | Block immediately, no prompt, no override |
| **ask** | Always prompt, even if mode would allow. User can override per-session. |

Rules are always scoped: `Tool(pattern)`.

```toml
[permissions]
deny = ["Bash(sudo *)", "Bash(rm -rf *)"]
ask = ["Bash(git push *)", "Bash(docker *)"]
```

No config `allow` rules — within-ceiling actions are already allowed by the mode. Use `ask` to add friction for specific commands; use `deny` to hard-block.

## Check flow

```
 1. Tool in agent's effective set? NO → blocked
 2. Classify action tier (read / edit / admin)
 3. Check deny rules → blocked (hard floor, not overridable)
 4. Check ask rules (config + not in session allows):
      policy.on_ask_rule = ask   → prompt
      policy.on_ask_rule = allow → skip the rule
      policy.on_ask_rule = deny  → blocked
 5. Resolve target path + zone
 6. If an explicit path grant covers the target and satisfies the tier → allowed
    (skill-declared grants short-circuit zone and ceiling checks)
 7. System zone + write/admin?
      policy.on_exceed = ask   → per-action prompt
      policy.on_exceed = allow → allowed
      policy.on_exceed = deny  → blocked
 8. Find effective mode for path:
      Within ceiling            → allowed
      Exceeds / no grant, and
        policy.on_exceed = ask   → prompt
        policy.on_exceed = allow → allowed
        policy.on_exceed = deny  → blocked
```

## Prompt options

**Actions within the mode ceiling on a granted path never prompt.** Three cases trigger prompts:

### Exceeds mode ceiling (home/temp zone)

1. Allow once
2. Switch to {mode} mode — persists `(current_path, mode)` in `permission.json`
3. Deny
4. Other...

After switching, all actions within the new ceiling pass without prompting.

### System zone writes

No mode switch. Always per-action:

1. Allow once
2. Deny

### Ask-rule override

Config `ask` rules force a prompt even within the ceiling. Example: `ask = ["Bash(git push *)"]` prompts in admin mode.

1. Allow once
2. Allow for this session — suppresses the ask rule for this session
3. Deny

## Session persistence

`~/.linggen/sessions/{session_id}/permission.json`:

```json
{
  "path_modes": [
    { "path": "~/workspace/linggen", "mode": "edit" }
  ],
  "locked": false,
  "allows": ["Bash:git push *"],
  "denied_sigs": ["Bash:rm -rf dist"]
}
```

`allows` stores ask-rule overrides — commands the user approved to suppress config `ask` rules for this session. `denied_sigs` stores tool calls the user denied (auto-blocked on retry).

No project-level `permissions.json`. Two persistent sources: `linggen.toml` (global) and session `permission.json` (per-session, cleared on session end).

## Session creators

Three ways to create a session, each with different initial permissions:

### User session

- Starts in **read** mode on the current directory.
- User upgrades mode interactively as needed.
- This is the default.

### Mission session

- Mode and path set by the mission creator (readonly / standard / full).
- Always **locked** — no prompts, pre-configured permissions only.
- Config deny rules still apply.
- Session promotion (user sends message) clears locked flag, resets to interactive.

### Skill invocation (within a user session)

Skills don't create sessions — they run inside the current user session. Skills that need elevated permissions declare it in frontmatter. Example from the `sys-doctor` skill:

```yaml
---
name: sys-doctor
description: System health analyst — scans disk, apps, caches, and system info
permission:
  mode: admin
  paths: ["/"]
  warning: "Sys Doctor runs diagnostic commands (df, du, sysctl, uname) and the AI may suggest cleanup commands."
---
```

When the user invokes the skill (click card, `/sys-doctor`, etc.):

```
Skill "sys-doctor" requests:
  admin mode on /
  ⚠️ Sys Doctor runs diagnostic commands (df, du, sysctl, uname)
    and the AI may suggest cleanup commands.

  [Approve]           ← grants (/, admin) in session permission.json
  [Run in read mode]  ← skill runs with current permissions, may fail
  [Cancel]
```

If approved, the grants are added to the **current session's** `permission.json`. The skill runs within the session with those grants. The grants persist in the session — user can revoke via the mode dropdown.

Skills without a `permission` section run with whatever the session already has.

## Remote access

| Context | Mode cap | Locked | Prompts |
|:--------|:---------|:-------|:--------|
| Local browser (owner) | Any | No | Yes |
| Remote same-user (owner) | Any | No | Yes |
| Remote different-user (guest) | Owner-set (default: read) | Yes | No |
| Proxy consumer (browser) | Room config ceiling | Yes | No |

Guest and consumer sessions are always locked — no prompts, actions within ceiling + allow rules proceed, everything else blocked. For proxy consumers, the room config (`allowed_tools`, `allowed_skills`) is the hard ceiling — the consumer's permission level operates within it.


## Subagents

- Inherit parent's session permission state (mode, path grants, session allows).
- Can only tighten. Cannot upgrade mode (no AskUser).
- Locked if parent is locked.

## Configuration

```toml
[agent]
tool_permission_mode = "read"    # Default mode for new sessions

[permissions]
deny = ["Bash(sudo *)", "Bash(rm -rf *)"]
ask = ["Bash(git push *)", "Bash(docker *)"]
```

## Future

- **Safety classifier**: model-based Bash classification for smarter auto-decisions.
- **OS sandbox**: Seatbelt (macOS) / bubblewrap (Linux) for defense-in-depth.
- **Hooks**: pre/post-tool-use hooks for programmatic permission decisions.
