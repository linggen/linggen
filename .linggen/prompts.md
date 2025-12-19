# Linggen Frequent Prompts

## Init my day

You are working with Linggen (local, multi-project memory + code index). Do the following steps:

1. List Linggen MCP tools and briefly explain when to use each.
2. Call `list_sources` to see available projects.
3. Call `memory.search` (keyword) with an empty query or broad query like "project" and limit 20 to load existing memories.
4. If you need deeper memory recall, also do a semantic pass:
   - Call Linggen’s semantic memory search tool (if available, e.g. `memory.search_semantic`). If not available, ask me to enable it.
5. Summarize “what Linggen knows” as:
   - Important conventions / decisions (from memory)
   - Active projects/sources and what they are
   - Any gaps you want me to confirm

Rules:

- Do NOT write or modify any files directly.
- If you think something should be saved as memory, propose a memory draft first and ask for my confirmation before calling `memory.create`.
- Always cite where each fact came from (memory id/title or source/file).

## Crystallize today’s engineering knowledge

Transform current insights, architectural decisions, and key learnings into persistent Linggen memory.

1. First, produce a “Memory Draft” with:

   - title (short)
   - tags (3–8)
   - scope (which source_id(s) this applies to, or cross-project)
   - body (markdown, concise but complete)
   - citations (source_id, file_path, line_range when applicable)
   - confidence (0.0–1.0)

2. Ask me: “Save this memory?” and wait for my explicit YES/NO.
   - If YES: call `memory.create` (or `memory.update` if we’re updating an existing memory) with the draft.
   - If NO: do not call any memory tool and do not write any file.

Rules:

- Absolutely DO NOT write `.linggen` files directly.
- If you are unsure about tags/scope/citations, ask me a single clarifying question instead of guessing.
