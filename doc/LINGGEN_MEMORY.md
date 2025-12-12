### Recommended Structure

- **TL;DR**

  - Project purpose
  - Top 3 rules

- **Do / Don’t (Hard Rules)**

  - Things that must always be respected
  - Things that should never be done again

- **Decisions**

  - Past decisions with:
    - Why
    - Tradeoffs
    - Scope / affected modules

- **Gotchas**

  - Symptom → Cause → Fix
  - Common pitfalls already discovered

- **Architecture Notes**

  - High-level module roles
  - Invariants and assumptions

- **Commands**

  - Common dev / test / build commands

- **Onboarding / Return Notes**
  - “If I come back in 3 months, start here”

This file is **not documentation for users**.
It is **memory for future collaborators and AI**.

---

## Pin to Memory (Core User Action)

Linggen does **not** expect users to write documents.

Instead, it focuses on **leaving lightweight memory traces**.

### Typical Flow

- Select code or text
- Run `Linggen: Pin to Memory`
- Write 1 short sentence (optional)
- Linggen records:
  - Text
  - File + line range
  - Timestamp
  - Type (decision / rule / gotcha / note)

Pins are:

- Stored as structured data (auditable)
- Rendered into `LINGGEN_MEMORY.md` for humans
- Exposed to Cursor via MCP for AI

---

## MCP Memory Injection (How AI Uses It)

Linggen exposes project memory via MCP as a **Memory Card**, not raw text.

A Memory Card typically contains:

- Relevant Do / Don’t rules
- Key past decisions (with reasons)
- Known gotchas
- Suggested clarifying questions
- Pinned code context

This allows Cursor’s agent to:

- Plan with constraints
- Avoid historical mistakes
- Ask better questions before acting

---

## What Linggen Is (and Is Not)

### Linggen Is

- A long-term memory layer
- Project- and developer-centric
- Local-first and privacy-preserving
- A complement to Cursor and LLMs

### Linggen Is Not

- A replacement for Cursor
- A general-purpose AI agent
- A documentation generator
- A cloud AI service

---

## Mental Model

> Cursor answers:  
> **“How should I do this now?”**
>
> Linggen answers:  
> **“Why did we do it this way before — and what should not change?”**

---

## Final Summary

Linggen solves a problem that LLMs structurally cannot:

**Persistence of intent, decisions, and constraints across time and projects.**

It does not make AI smarter.
It makes AI **less forgetful**.
