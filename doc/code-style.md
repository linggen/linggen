---
type: spec
reader: Coding agent and users
guide: |
  The rules every change to this codebase follows — architecture first, then style.
  Keep it brief. State the rule and why; no code snippets, no history.
---

# Architecture Principles and Code Style

Most of this code is written by AI agents and steered by a person. Both must
be able to read it. An agent starts every session with no memory of the
system: the structure is the memory it does not have. A person who cannot
read the structure cannot steer it. So the measure of a design is not how
small its pieces are — it is **in how many places one thing is decided**.

## Architecture Principles

### 1. Readable by people and by agents

- Easy to read, to maintain, to extend. Plain beats clever.
- Names a search finds; calls a reader can follow. Indirection that hides
  what runs next costs an agent as much as it costs a person.

### 2. One thing is decided in one place, and a machine holds it there

- A rule lives at the one point every path passes through. Fix a rule
  there, never entrance by entrance.
- Lock the structure: a private field, a single entry, an exhaustive
  `match`, or a test that fails when a second copy appears. A convention
  nobody enforces is forgotten by the next session.
- Before adding a third thing of a kind, or changing several modules: count
  the existing ones, and name the one place that should decide.

### 3. Open what varies, close what does not

- A set others add to — skills, model providers, tools, settings sections —
  is a declared list plus a registry. Adding one changes no existing code.
- A closed set — permission levels, who started a turn, kinds of session —
  is one enum matched in one place, so the compiler finds the case that was
  missed.
- A design pattern is a means. Use one when you can name the thing that
  varies and that it encapsulates; if you cannot, do not.

### 4. Sound abstractions, the changeable part enclosed, human names

- An abstraction stands for one idea a person would name the same way.
- What may change sits behind one boundary; callers do not know how.
- A name says what a thing is for, not how it is built.

### 5. Shared logic is called, not copied

- The test is "will these change together", not "do these look alike".
- Extract at the third occurrence. While there are two, leave them and let a
  comment at each point to the other.

### 6. When one piece of logic must span files, say so where it is read

- First try not to spread it.
- If it must be spread, every site carries a comment that points to the
  place that decides.
- For a rule that matters, add a lock (principle 2). Comments go stale and
  agents believe them.

### 7. Encapsulate to remove a decision, not to remove lines

- Less code is a result, not a goal. A type, a test and a line of "why" are
  worth their lines.
- Between two forms of similar length, choose the plainer.

### 8. A claim carries its evidence

- "Fixed" and "works" mean it was run and seen. A conclusion from reading
  code is said as that.
- A report says what was changed, what was verified, and what was not.

## User-Facing Brevity (global)

Every user-facing message — UI copy, status lines, widget results,
notifications — says what happened in **≤20 words**. No preamble, no
justification, no recap. If it needs more, link or make it expandable;
the default view is one glanceable line. Humans are reading; respect the
reader's time.

## File and Function Size

- Size is a signal, not the rule. A long file that holds one decision is
  fine; a short one that shares a decision with three others is not.
- Split along responsibilities — when a file or function has come to hold
  several decisions — never only to make it shorter.
- Keep functions focused on one responsibility.
- Refactoring is expected, not optional, when complexity grows. An agent
  feels no pain adding the next branch, so the trigger is the rule below,
  not discomfort.

## Flat Logic First

- Prefer guard clauses and early returns.
- Avoid deep nested `if/else` and nested `match` chains where flattening is possible.
- Extract nested logic into helper functions with clear names.
- Keep async control flow explicit and easy to trace.
- A long conditional chain over a closed set becomes one `match` on an enum,
  in one place; over an open set, a declared list and a registry
  (principle 3). The same chain written twice is the real fault.
- Name functions to describe their purpose (e.g. `build_consumer_prompt`, `build_owner_prompt` instead of one function with mode flags).

## Clean Code Policy

- Remove unused code, old branches, and stale utilities.
- Do not keep compatibility/fallback code when current logic is confirmed and stable.
- Keep reliability fallbacks that are required for system safety and operations (for example offline startup, retry, rollback, and recovery paths).
- Remove dead feature flags and temporary migration paths after rollout is complete.
- Keep imports, structs, and modules free of unused fields and APIs.

## Refactoring Triggers

- The same thing is decided in more than one place.
- Repeated logic appears a third time.
- Function has multiple unrelated responsibilities.
- Nested flow makes behavior hard to reason about.
- A change requires touching too many unrelated lines.

## Review Checklist

- Is there one place that decides this, and does a lock hold it there?
- Would a new case of this kind need existing code changed? Should it?
- Can a person follow it without being told the history?
- Can nesting be flattened?
- Is there unused or obsolete code that should be deleted?
- Does the report separate what was verified from what was not?
