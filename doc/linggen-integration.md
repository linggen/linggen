# Linggen Integration

This doc summarizes how Linggen integrates with a repo and how install paths are resolved.

## Install Path Resolution

### Default (no flags)

Both `linggen init` and `linggen skills add` use the same root resolution logic:

1) Walk upward from the current working directory to find a `.git` folder.
   - If found, that directory is the repo root.
   - If `.claude/` does not exist at the repo root, it is created.
2) If no `.git` is found, walk upward to find a parent `.claude` folder.
   - If found, use that folder's parent as the root.
3) If neither is found:
   - `linggen init` falls back to global install paths.
   - `linggen skills add` falls back to `~/.claude/skills/<skill>`.

### Flags

- `linggen init --local`: uses the current working directory directly.
- `linggen init --global`: uses global install paths (home/CODEX_HOME), no repo lookup.

## Integration Folders and Files

### Skills directories

- `.claude/skills/<skill>`
  - Local repo install for Claude/Cursor-style skills.
  - Created at repo root on demand.

- `.codex/skills/<skill>`
  - Local repo install for Codex skills.
  - If global and `CODEX_HOME` is set, uses `$CODEX_HOME/skills`.
  - Otherwise uses `~/.codex/skills`.

### Repo entrypoints

When `linggen init` runs in a repo (non-global), it bootstraps these files:

- `CLAUDE.md`
  - Ensures it includes a pointer to `.claude/skills/linggen/SKILL.md`.

- `AGENTS.md`
  - Mirrors the contents of `CLAUDE.md`.

- `.cursor/rules/linggen.md`
  - Written from `.claude/skills/linggen/SKILL.md` if it exists.

### Linggen project knowledge

- `.linggen/`
  - Project-local knowledge store (memory/policy/skills).
  - Not created by the CLI here, but treated as a source of truth by Linggen tooling.
