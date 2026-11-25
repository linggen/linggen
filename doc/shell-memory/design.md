# Shell memory design

Shell memory (formerly Memory V2) is an experimental cross-agent memory shell. Each agent owns its memory; Linggen reads other agents' files in place and forwards an index, but never writes, imports, tidies or summarises those files. Linggen writes only its own notes. See [results and decision](results-and-decision.md) for why main retains Linggen memory instead.

## Own notes and one writer

Linggen's own store is plain Markdown under `~/.linggen/memory/`, organised by person, area, project, moments and chronicle. Topic files contain entries with a summary, kind, scope, voice and dates. The engine generates the index from the notes. `memory_notes` provides index, read, search and write operations; the engine's one write path validates writes, keeps history and checks versions. User-voice entries require the owner's agreement before alteration. Notes outside the index remain searchable but depend on an agent deciding to look.

The audience fence in `crates/linggen-engine/src/own_memory/fence.rs` prevents private areas, moments and the chronicle from reaching coding hosts. The chronicle writer has a separate, restricted audience. The fence applies to Linggen's own notes; other agents' material is separately handled by the outside-memory reader and redaction path.

## Session start and readers

At session start, a bounded block contains the owner's rules and topic summaries from Linggen, followed by source-labelled lines from other agents. Global sources are available globally; project sources stay within their project chain and are never promoted to global. The combined block is limited to 200 lines and 16 KiB. Note bodies are read or searched on demand; Shell memory does not inject semantic recall per turn. An agent is not sent its own memory back.

Reader declarations are extensible through trusted skills, with embedded defaults in `crates/linggen-engine/src/outside_memory/readers/`. The engine owns path resolution, parsing, scoping and bounded output. Missing or unknown formats are reported rather than treated as an empty memory without explanation.

- **Claude Code:** a project's `MEMORY.md` index and linked notes, plus global instructions. Project notes remain project-scoped.
- **Codex:** `memory_summary.md`, ad-hoc notes and global instructions; detail files are searchable. The declarations cover `memories/` and `memories_v2/`, choosing the more recently changed summary. The v2 format was not verified against a real v2 folder in the design investigation.

The entry points are `crates/linggen-engine/src/outside_memory/`, `crates/linggen-engine/src/own_memory/forward.rs`, and the `memory_agents` and `memory_notes` tools. The plugin connects outside hosts at session start.

## Chronicle and maintenance

The monthly chronicle records a dated longer-term history in Linggen's own Markdown store, with yearly summaries. The mission prepares inputs from own notes, Linggen sessions and other agents' note lines, then writes through the same notes writer. Other agents' session logs and entry-by-entry phone sync were not included in the initial build. See `missions/chronicle/mission.md` and `crates/linggen-engine/src/own_memory/chronicle.rs`.

A **weekly tidy was agreed as a design, not established here as built**: when idle, process only new or changed Linggen notes, merge duplicates, mark superseded agent notes and flag preferences whose scope exceeds their evidence. Changes to the owner's words require approval. It would never tidy other agents' memory or add per-turn recall.
