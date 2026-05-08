---
type: spec
reader: Coding agent and users
guide: |
  Product specification — describe what the system should do and why.
  Keep it brief. Aim to guide design and implementation, not document code.
  Avoid implementation details like function signatures, variable types, or code snippets.
---

# Skills

A **skill** is the unit of extension in Linggen. Drop a folder under `~/.linggen/skills/` and your assistant gains a new capability — a memory store, a system diagnostic, a chess opponent, an X bot. No code changes, no SDK, no hosted plugin server.

This is what makes Linggen a platform, not a single product. The same agent loop that chats with you also powers Sys Doctor, ling-mem, and game-table — every "AI app" Linggen hosts is a skill on top of one shared runtime.

## What you can do with skills

- **Install one** from the marketplace: `/skiller add weather` and your agent can answer weather questions.
- **Invoke one** in chat: type `/sys-doctor` to launch the system health dashboard, or just ask "scan my disk" and the agent finds and runs the skill on its own.
- **Stack them** in a single conversation — the agent reads memory through `ling-mem`, scans disk through `sys-doctor`, and posts to X through `xbot`, all in the same chat. The skills don't know about each other; the agent loop composes them.
- **Make your own** by writing a `SKILL.md`. If you can write a markdown note, you can write a skill.

## Three flavors of skill

| Flavor | What it does | Example |
| :----- | :----------- | :------ |
| **Instructions** | Gives the agent rules and context for a topic. The body shapes how the agent responds; tools stay the same. | `linggen-guide` (documentation Q&A) |
| **App** | Has its own UI in an embedded panel. The agent drives the UI through `PageUpdate` data tools. | `sys-doctor`, `game-table`, `arcade-game` |
| **Service** | Backs an engine-defined capability (currently memory; later search, vcs, notifications). The model uses generic tool names; the skill is a swappable backend. | `ling-mem` |

Most skills are the first kind. App skills are how you ship full AI apps with custom UI. Service skills are how Linggen lets you swap a built-in capability without the model noticing.

## How skills get invoked

- **The user types `/skill-name [args]`** in chat — works for any user-invocable skill.
- **The model decides** — if the skill's `description` matches the conversation, the model invokes it on its own. Skills can opt out via `disable-model-invocation: true`.
- **Bound to a session** — interactive app skills create a session that has the skill active for every message in it.

## Why skills exist (system-level)

The skill is the **unit of distribution and the unit of composition** in Linggen. It's what turns the runtime from a single product into a platform:

- **Distribution** — a skill is a folder. Write one, drop it in `~/.linggen/skills/`, share it, install someone else's. No registry account, no plugin signing, no per-project config files.
- **Composition** — a session can pull in any subset of installed skills. The agent loop chooses based on the conversation; users can also force a skill with `/skill-name`.
- **Identity** — a skill defines what an "AI app" *is* in Linggen. Sys Doctor isn't hard-coded; it's a `SKILL.md` plus a few scripts. The runtime treats first-party and third-party skills identically.

This is what lets the same runtime power "your personal assistant" out of the box and "an AI app you built last weekend" with no architectural change. The dual framing in the product vision (app engine + assistant) is *enabled* by the skill system.

### Design goals

| Goal | What it enables |
| :--- | :-------------- |
| **Zero-code extension** | A non-coder can write a useful skill in markdown. The bar to extend Linggen is reading and writing English. |
| **Self-describing** | Each skill carries its own `description`; the model reads it and decides whether to invoke. No out-of-band registration. |
| **Portable** | Same directory works in Linggen, Claude Code, and Codex via the open [Agent Skills](https://agentskills.io) standard — the ecosystem isn't owned by any one vendor. |
| **Progressive disclosure** | Context is precious. Metadata for all skills, body only when active, resources only when read. The agent stays sharp with hundreds of skills installed. |
| **One primitive, many shapes** | Transient invocation, session-bound, iframe app, long-running daemon. Same `SKILL.md`, different deployments. |
| **Swappable backends** | Service skills implement engine-defined capabilities. Users can replace the backend; the model never sees a difference. |
| **Permission isolation** | Each skill declares its permission ask up front; activation grants exactly the listed paths and modes. |

## Related docs

- `session-spec.md` — session-bound skills, effective tools.
- `tool-spec.md` — built-in tools (the kernel API).
- `agent-spec.md` — how agents use skills.
- `permission-spec.md` — what `permission:` does at activation.
- `memory-spec.md` — first capability example.

## Format

Each skill is a directory with `SKILL.md` as entrypoint:

```
my-skill/
├── SKILL.md           # Main instructions (required)
├── references/        # Detailed docs, loaded on demand
├── scripts/           # Executable code the model can run
└── assets/            # Static resources (templates, missions, schemas)
```

`SKILL.md` is YAML frontmatter + markdown body. Directory name should match `name`.

### Progressive disclosure

1. **Metadata** (~100 tokens): `name` + `description` loaded at startup for all skills.
2. **Instructions** (< 5000 tokens): full `SKILL.md` body loaded on activation.
3. **Resources**: files in `scripts/`, `references/`, `assets/` loaded only when needed.

Keep `SKILL.md` under 500 lines.

## Frontmatter

Three groups of fields. Standard fields work across tools; the others are extensions.

### Agent Skills standard

| Field | Purpose |
| :---- | :------ |
| `name` | Lowercase, hyphens, ≤64 chars. Becomes `/slash-command`. |
| `description` | What the skill does and when to use it. Model reads this to decide. |
| `allowed-tools` | Tools permitted when the skill is active. |

### Claude Code extensions (also supported)

| Field | Purpose |
| :---- | :------ |
| `argument-hint` | Autocomplete hint, e.g. `[issue-number]` |
| `disable-model-invocation` | `true` = user-only |
| `user-invocable` | `false` = model-only |
| `model` | Preference: `cloud`, `local`, or specific ID |
| `context` | `fork` = run in isolated subagent |
| `agent` | Subagent type when `context: fork` |

### Linggen extensions

| Field | Purpose |
| :---- | :------ |
| `trigger` | Custom prefix, e.g. `"!!"` |
| `app` | Makes the skill a runnable app with its own UI |
| `tools` | Custom tools the skill exposes to the agent (see "Custom tools") |
| `permission` | Permission request, prompted at activation |
| `cwd` | Starting cwd for sessions invoking this skill |
| `install` | Script that runs once on installation |
| `provides` / `implements` | Marks the skill as a service backend for an engine-defined capability |
| `requires` | External dependencies to resolve at install |

## Custom tools

A skill can declare its own tools in the `tools:` frontmatter list. Each tool surfaces to the agent under the skill's namespace and dispatches according to which fields are populated:

| Kind | Trigger | What happens when the agent calls it |
| :--- | :------ | :----------------------------------- |
| **Shell** | `cmd: "..."` is set | Engine runs the command, stdout returns to the model. `$SKILL_DIR` and `{{argname}}` placeholders are expanded. |
| **HTTP** | `endpoint: "..."` is set (requires `daemon:` block) | Engine POSTs args as JSON to `http://127.0.0.1:<daemon.port>{endpoint}`; response body returns to the model. |
| **Data** | Neither `cmd` nor `endpoint` | No backend execution. Args surface as a `content_block` event to the app's iframe. Used for app UI signals like `PageUpdate`. |

### Schema

```yaml
tools:
  - name: ScanDisk                          # Required. Tool name the agent calls.
    description: "Run a fresh disk scan."   # Required. Tells the agent when to use it.
    cmd: "$SKILL_DIR/scripts/scan-disk.sh"  # Shell tool
    tier: read                              # read | edit | admin (default: admin)
    timeout_ms: 30000                       # default: 30000
    args:
      target:
        type: string                        # string | object | array | number | boolean
        required: true
        default: "~"
        description: "Path to scan"
        items: { type: object }             # for arrays only
    returns: "Sectioned text output."       # Optional, hint for the model
```

### Permission tier

Shell and HTTP tools obey the skill's `permission.mode`. A `tier: read` tool runs ungated when the skill is in read mode; `tier: edit` or `tier: admin` prompts the user. Data tools have no side effect to gate, so `tier` is ignored. Omitting `tier` defaults to `admin` (the strict default).

### Reuse across kinds

App skills automatically receive a built-in `PageUpdate` data tool — you do not declare it. Service-backend skills (those with `implements:` for an engine-defined capability like `memory`) inherit the engine's tool names and schemas; their declared `tools:` are private extensions on top of the capability's contract.

Tool definitions are parsed once at skill-load time. Editing `tools:` requires a server restart to register; editing scripts pointed to by `cmd:` does not.

## Service skills

Most skills are "instructions + tools." A few skills are **service backends**: they implement a named **capability** that the engine defines (e.g. `memory`). The engine owns the tool names, schemas, and permission tiers; the skill only declares which capability it provides and where its daemon lives.

The first capability is `memory` — see `memory-spec.md`. Future capabilities may include `search`, `vcs`, `notifications`.

The point is **swappability**: two memory skills expose identical `Memory_*` tools to the model because both conform to the same engine-defined contract. Users can switch backends without the model seeing any change.

A skill can also ship its own private tools (unique to itself) alongside any capabilities it implements.

## App skills

A skill with an `app:` section is a runnable app — invoking it opens a UI.

| Kind | Model involvement | Example |
| :--- | :---------------- | :------ |
| **Standalone** | None — pure frontend | `arcade-game` (Snake, Pong, Tetris) |
| **Interactive** | App UI talks to the agent via the session API | `game-table`, `sys-doctor` |

Three launcher types: `web` (static files in an embedded panel), `bash` (run a script, stream output), `url` (external URL).

Interactive apps are **session-bound** — every message in the session activates the skill (tool restrictions, prompt injection). The app talks to the agent through the same HTTP/WebRTC surface as the main UI; no custom endpoints needed.

Every app skill receives a built-in `PageUpdate` data tool — the agent calls it whenever state the user should see changes, and the iframe re-renders. Each app defines its own page layout schema in its SKILL.md.

### App-mode (`?app_mode=1`)

When a skill page is hosted by a Linggen app shell (e.g. SysDoctor.app, LingMem.app), the shell appends `?app_mode=1` to the URL and drives shell-level UI affordances via `window.postMessage`. The skill is otherwise unchanged — the same files run in the public web UI without app_mode and behave identically.

Skills that want to be hostable by an app shell ship a tiny `app-mode.js` that:

1. No-ops if `URLSearchParams(location.search).get("app_mode") !== "1"`.
2. Listens on `window.message` and handles:
   - `{ type: "linggen:show-settings" }` → overlay `settings.html` over the page.
   - `{ type: "linggen:hide-settings" }` → hide the overlay.

The shell never reaches into skill DOM; it only posts these messages. Reference: `sys-doctor/scripts/app-mode.js`.

## Install

Skills can declare an `install` field pointing to a script that runs once when the skill is installed. Used to seed directories, copy mission files into `~/.linggen/missions/`, fetch binaries via `requires:`, or perform any other one-time setup.

```yaml
install: scripts/install.sh
```

Scripts run with `$SKILL_DIR` set to the skill directory and must be **idempotent**. The same hook runs on every install path — `ling init`, the WebUI Install button, `ling skills install`, and auto-install on first startup.

Missions ship as files under the skill's `assets/` and are copied by the install script — there is no `mission:` frontmatter field.

## Permissions

Skills can declare a `permission` request to ask for elevated access at activation. The user is prompted before the skill runs. **Each path declares its own mode** — there is no top-level default. A skill can ask for `read` on one path and `write` on another in a single block.

```yaml
permission:
  paths:
    - { path: ~/.linggen/skills/pulse, mode: write }
    - { path: /tmp,                    mode: read }
  warning: "Pulse writes session JSON inside its own data dir; reads /tmp."
```

`mode` is `read`, `edit` (alias `write`), or `admin`. On approval, each `(path, mode)` pair is added to the session's grants — the cwd is not silently broadened. See `permission-spec.md`.

## Skill tools

Beyond capability tools, a skill can register its own:

- **Command tools** — execute a shell command with template substitution. Same gating as `Bash`.
- **Data tools** — pass structured data from the agent to the app UI (no shell, no side effects). The engine emits a `content_block` event; the app iframe receives it and re-renders. Enables real-time structured updates without text-tag parsing.

Skill tools are available only while the skill is active (transient invocation or session-bound).

## Activation modes

| Mode | Trigger | Scope |
| :--- | :------ | :---- |
| **Transient** | `/skill-name` in chat | Single invocation, then restored |
| **Session-bound** | Session created with `skill:` field | Entire session |

Session-bound effective tools are `intersection(agent.tools, skill.allowed-tools)`. When that intersection is empty, tool-related prompt sections (schemas, delegation, plan mode) are skipped — the agent runs as a pure conversational skill.

## Discovery

Loaded at startup and on file change (live reload).

| Priority | Path | Scope |
| :------- | :--- | :---- |
| 1 | `.linggen/skills/<name>/SKILL.md` | Project (highest) |
| 2 | `~/.linggen/skills/` | Global personal |
| 3 | `~/.claude/skills/`, `~/.codex/skills/` | Cross-tool compat |

All skill descriptions are included in agent context so the model knows what's available.

## Invocation

- **User** — type `/skill-name [args]` in chat.
- **Model** — invokes based on description match.

`disable-model-invocation: true` makes a skill user-only; `user-invocable: false` makes it model-only.

## Trigger symbols

Parsed from user input only (never from model output):

- `/` — built-in commands and skill invocation.
- `@` — file mentions.
- Custom triggers declared in frontmatter.

Matching order: system triggers → user-defined triggers → pass-through.

## Cross-tool compatibility

Same skill directory works in Linggen, Claude Code, and Codex. Linggen-specific fields (`trigger`, `app`, `provides`, `implements`) are ignored by other tools; everything else is shared. Skills installed under `~/.claude/skills/` or `~/.codex/skills/` are discovered by all three tools automatically.
