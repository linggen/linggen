---
type: spec
reader: Coding agent and users
guide: |
  Product specification — describe what the system should do and why.
  Keep it brief. Aim to guide design and implementation, not document code.
  Avoid implementation details like function signatures, variable types, or code snippets.
---

# Code Style Specification

## Goals

- Keep code easy to read, maintain, and refactor.
- Keep files and functions at a manageable size.
- Prefer simple, flat control flow over deep nesting.
- Keep the codebase clean by removing dead and legacy-only code.

## User-Facing Brevity (global)

Every user-facing message — UI copy, status lines, widget results,
notifications — says what happened in **≤20 words**. No preamble, no
justification, no recap. If it needs more, link or make it expandable;
the default view is one glanceable line. Humans are reading; respect the
reader's time.

## File and Function Size

- Keep source files at a reasonable length; split large files into focused modules.
- Keep functions short and focused on one responsibility.
- If a file or function becomes long or hard to navigate, refactor early.
- Refactoring is expected, not optional, when complexity grows.

## Flat Logic First

- Prefer guard clauses and early returns.
- Avoid deep nested `if/else` and nested `match` chains where flattening is possible.
- Extract nested logic into helper functions with clear names.
- Keep async control flow explicit and easy to trace.
- Use design patterns (dispatch, composition, traits) instead of long conditional chains.
- Name functions to describe their purpose (e.g. `build_consumer_prompt`, `build_owner_prompt` instead of one function with mode flags).

## Clean Code Policy

- Remove unused code, old branches, and stale utilities.
- Do not keep compatibility/fallback code when current logic is confirmed and stable.
- Keep reliability fallbacks that are required for system safety and operations (for example offline startup, retry, rollback, and recovery paths).
- Remove dead feature flags and temporary migration paths after rollout is complete.
- Keep imports, structs, and modules free of unused fields and APIs.

## Refactoring Triggers

- Repeated logic appears in multiple places.
- Function has multiple unrelated responsibilities.
- Nested flow makes behavior hard to reason about.
- A change requires touching too many unrelated lines.

## Review Checklist

- Is the file still easy to scan?
- Is the function size still reasonable?
- Can nesting be flattened?
- Is there unused or obsolete code that should be deleted?
