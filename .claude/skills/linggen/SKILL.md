---
name: linggen
description: Expert assistant for the Linggen ecosystem. Use when searching the codebase, enhancing prompts with context, managing project memory, or querying the global library. Trigger when the user mentions "search", "indexing", "memory", "policies", or "linggen".
allowed-tools: [Bash, Read]
---

# Linggen Expert Skill

You are an expert at using Linggen's internal tools to provide deep context and maintain architectural standards.

## Core Workflows

### 1. Codebase Discovery & Search
When you need to find where a feature is implemented or find specific code patterns:
- **Search chunks:** `bash scripts/search_codebase.sh "<query>" [strategy] [limit] [source_id]`
- **Deep search (metadata):** `bash scripts/query_codebase.sh "<query>" [limit] [exclude_source_id]`
- **List sources:** `bash scripts/list_sources.sh` (use this if you don't know the `source_id`)

### 2. Prompt Enhancement
To get a fully context-aware prompt that includes intent detection and applied user preferences:
- `bash scripts/enhance_prompt.sh "<query>" [strategy] [source_id]`

### 3. Project Memory
Linggen memories capture architectural decisions and constraints.
- **Search memories:** `bash scripts/memory_search_semantic.sh "<query>" [limit] [source_id]`
- **Fetch by Anchor:** If you see `//linggen memory: <ID>` in code, run:
  `bash scripts/memory_fetch_by_meta.sh "id" "<ID>"`
- **Local Read:** You can also read memories directly from `.linggen/memory/` if you are in the target repo.

### 4. Global Library (Skills & Policies)
Linggen maintains a global library of behavioral skills and architectural policies.
- **Browse library:** `bash scripts/list_library_packs.sh`
- **Read pack:** `bash scripts/get_library_pack.sh "<pack_id>"` (e.g. `skills/linggen/SKILL.md`)

## Operational Guidance
- **Health Check:** If the server feels slow or unresponsive, run `bash scripts/get_status.sh`.
- **Token Efficiency:** Prefer `search_codebase` for quick lookups and `enhance_prompt` for complex architectural questions.
- **Cross-Project:** Most search tools support searching across all indexed projects if `source_id` is omitted.
