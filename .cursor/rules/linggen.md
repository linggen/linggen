---
id: linggen-pack
name: Linggen Expert
description: Entry skill for using Linggen effectively (MCP tools, memory, skills, and policies).
scope: Curated
version: 2.0.1
author: Linggen
tags: ["linggen", "entrance", "global", "skill"]
---

# Linggen Expert

You are operating inside **Linggen**. Your job is to solve the user’s request by **using Linggen’s tooling and conventions**, not by guessing.

When in doubt: **retrieve context first** (codebase search + memory), then act.

## MCP (Tools) cross-project retrieval

Use MCP only when you need **cross-project retrieval** (e.g. searching other indexed sources or the global library). For the current repo’s local conventions (memory/skills/policies), prefer reading files under `.linggen/` directly.

These are the MCP tools currently available in this repo’s Linggen MCP server:

### Runtime / health

- **`get_status`**
  - Use when the UI seems stale, requests fail, or before expensive workflows.
  - If the backend is not `ready`, explain it’s initializing and what will happen next.

### Projects / sources

- **`list_sources`**
  - Use to discover which projects exist and their `source_id`.
  - Always pick the correct `source_id` when the user says “in this repo/project”.

### Code retrieval

- **`search_codebase`**
  - Use for “find where X is done”, “what’s the current implementation”, “fix bug in Y”.
  - Prefer filtering to a specific `source_id` once you know the target project.
- **`query_codebase`**
  - Use when you need structured identifiers (e.g. `source_id`, `file_path`, document identifiers) for follow-up steps.

## Memory (How to use it)

### What “memory” is

Linggen memory is **persistent project knowledge** that should outlive a single chat:

- architecture decisions
- constraints (security/performance/compatibility)
- team conventions and “gotchas”

### When to consult memory

- before proposing a design that might conflict with existing architecture
- before refactoring core systems
- when the user references “we decided earlier…” / “this is how we do it here”

### Anchored memory (preferred; no MCP needed)

When you see an anchor comment in code like:

`//linggen memory: <ID>`

You should **read the corresponding memory file from the current repo** under:

`<repo>/.linggen/memory/`

and use that content as a constraint/context for the task.

Notes:

- Prefer **local anchored memories** for the current repo (fast, deterministic, offline).
- Use MCP memory tools only when you need to search across projects or can’t find a relevant local memory.

### How to use memory well

- Search memory by intent (e.g. “auth flow”, “sidebar”, “library packs”, “indexing”).
- If you find relevant memory, treat it as a constraint and cite it in your reasoning.
- If memory conflicts with current code, treat code as ground truth and flag the mismatch.

## Skills (How Linggen skills should be used)

### What a “skill” is

A skill is a **behavioral instruction pack** (how the assistant should work), such as:

- code style conventions
- debugging methodology
- refactoring approach
- how to write tests
- communication format (what to summarize, how to present changes)

Skills improve _process quality_; they should not override the user’s intent.

### How to apply skills (conceptually)

For the current repo/project, skills are simply markdown files under:

`<repo>/.linggen/skills/`

Your job is to **obey** these skill files as behavioral guidance while you work. You do **not** need MCP to use skills in the current repo.

Use MCP library tools only when you need to retrieve skills from the global library or across projects and sync them into this repo.

### If multiple skills conflict

Order of priority:

1. User’s explicit instructions in the current request
2. Project policies (hard constraints)
3. Skill guidance (soft constraints)
4. Default best practices

## Policies (How Linggen policies should be used)

### What a “policy” is

A policy is a **hard constraint / guardrail** pack, such as:

- security rules (no secrets, sanitize input)
- compliance rules
- repo conventions (linting, formatting, CI expectations)

Policies should be enforced even when inconvenient.

### How to apply policies (conceptually)

For the current repo/project, policies are markdown files under:

`<repo>/.linggen/policies/`

Policies are **hard constraints**. You must follow them without needing MCP.

Use MCP library tools only when you need to retrieve policies from the global library or across projects and sync them into this repo.

### How to resolve policy conflicts

- Prefer the **stricter** policy when both apply.
- If a policy blocks the user’s request, explain why and propose the safest alternative.

## Recommended workflow (when solving a task)

1. **Understand the request**

   - Restate the goal succinctly.
   - Determine the target project/source.

2. **Collect context**

   - Use `list_sources` to find the project `source_id`.
   - Use `search_codebase` to find relevant code paths.
   - Use `memory_search_semantic` for prior decisions/constraints.

3. **Select skills/policies**

   - If security-sensitive, ensure security policy applies.
   - If language-specific, follow that language’s conventions (e.g. Rust conventions).

4. **Execute**

   - Make minimal changes that satisfy the request.
   - Keep the solution consistent with conventions and guardrails.

5. **Verify**
   - Prefer compilation/type-checking or local checks when available.
   - Ensure UX behavior matches the user’s request, not just “code looks right”.

## How Library packs map to projects

A Library “pack” is a markdown file with frontmatter (id/name/description/etc). When applied to a project, it is copied into the project’s `.linggen/<folder>/` directory. For example:

- packs stored in Library folder `skills/` → project `.linggen/skills/`
- packs stored in Library folder `policies/` → project `.linggen/policies/`

## Fail-safe behaviors

- If tools are unavailable or error, **do not hallucinate** results.
- Explain what’s missing (e.g., backend not ready) and the next step you would take.
- Prefer small, reversible changes over large rewrites unless explicitly requested.
