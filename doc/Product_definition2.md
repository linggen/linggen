## Linggen Architect – Short Product Definition

### 1. Product vision

- **Vision**: Become the **system-definition layer above LLMs** – the place where developers and architects define how a system should work, so tools like Cursor or Claude can reliably implement and evolve it.
- **Role in the toolchain**: IDE/LLM (Cursor, Claude, Copilot) are where you **write and edit code**. Linggen is where you **understand the existing system and specify the next version** in a structured, machine-consumable way.
- **Form factor**: A **local-first, code-aware architecture IDE** with Obsidian-like usability and exportable specs for LLMs.

---

### 2. Target users & core jobs

- **New engineers on a large codebase**
  - "Show me the shape of this system and where to start."
- **Senior developers / maintainers**
  - "Before I change this module, tell me what depends on it and what might break."
- **Architects / tech leads / system designers**
  - "I want a single workspace to design, document, and enforce architecture, and then drive LLMs from that spec."

Linggen should:

- Shorten **onboarding time**.
- Reduce **risk of unintended side-effects** from changes.
- Provide a **single, living specification** that connects docs, structure, and code.

---

### 3. Core concept – System definition layer for LLMs

- Linggen maintains a **code-backed model of the system**:
  - File/module dependency graph.
  - Components/services and their responsibilities.
  - Interfaces (endpoints, events, data models) and key constraints.
- On top of this, users create **human-readable design docs** (markdown) and **lightweight structured specs** (YAML/JSON-like definitions).
- Linggen can then **export LLM-ready briefs**, e.g.:
  - "Implement `UserService` according to this spec and these dependencies."
  - "Apply this schema change through all layers described in the spec."

LLMs remain the code generator; Linggen becomes the **source of truth for what should exist and how it should fit together**.

---

### 4. Current product (v1) – File dependency graph

**Goal**: Give developers a fast, accurate way to see **how files and modules depend on each other**, and use that as the foundation for system specs.

- **Analysis**

  - Use Tree-sitter to build a **file-level dependency graph** for supported languages (Rust today; TS/JS, Go, Python next).
  - Nodes: files/modules with metadata (`id`, `label`, `language`, `folder`).
  - Edges: static "uses/imports" relationships (`import`, `use`, `mod`, etc.).
  - Graphs are cached per source and can be rebuilt on demand.

- **API & UI**
  - Backend endpoints to get **graph status**, **graph data**, and **trigger rebuilds**.
  - Frontend `GraphView` renders an Obsidian-like, zoomable dependency graph with:
    - Search by file name.
    - Filtering by folder.
    - Hover/selection to show local neighborhoods and connections.

This gives users a **live system map** they can trust before any higher-level spec or design work.

---

### 5. Next steps (v2) – Design workspace & specs

- **Design notes (markdown)**

  - Built-in editor for system / component design docs.
  - Notes can link to graph nodes (files, folders, components) and vice versa.

- **Lightweight structured specs**

  - Simple schemas for components, interfaces, data models, and constraints.
  - Stored alongside notes and linked to real code via the graph.

- **LLM exports**
  - Generate **prompt bundles / briefs** for specific components or views:
    - Include structured spec + selected code context + relevant design notes.
  - Designed to be pasted into Cursor/Claude today; later, integrate via APIs.

---

### 6. Non-goals (for now)

- Linggen is **not** a full UML / enterprise modeling suite.
- Linggen is **not** a general-purpose wiki or note app.
- Linggen does **not** replace the IDE; it **guides** IDE+LLM work by providing a shared, structured understanding of the system.

---

### 7. Success criteria

- Developers and architects **keep Linggen open** while reading, designing, or reviewing systems.
- Teams start **from Linggen specs** when asking LLMs to implement or refactor features.
- New engineers report that the **graph + design workspace** helped them understand a service materially faster than reading code alone.
