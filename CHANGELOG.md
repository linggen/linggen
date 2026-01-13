# Changelog

## [0.5.0] - 2026-01-13

### Added

- **Library System**: Introduced a new library template system with predefined skills (linggen, code-simplifier, react-pack) and policies.
- **Library View**: Added a dedicated view for exploring and managing library packs.
- **MCP Support**: Implemented Model Context Protocol (MCP) handlers in the backend.
- **Activity View Enhancements**: Improved activity monitoring and logging for better visibility into background tasks.

### Changed

- **Tailwind CSS Migration**: Major frontend refactor migrating from custom CSS files to Tailwind CSS for better consistency and performance.
- **Theme Overhaul**: Updated dark and light themes with a more refined, Obsidian-like color palette.
- **Editor Improvements**: Enhanced the CodeMirror 6 editor with better live preview rendering and mermaid diagram support.
- **Sidebar & Navigation**: Redesigned the sidebar for better source management and more intuitive navigation.

### Fixed

- **Editor Visibility**: Fixed a contrast issue where inline code keywords (like `function`) were nearly invisible in dark mode.
- **Rescan Reliability**: Improved path handling and ownership checks in the internal indexer.

## [0.4.0] - 2026-01-02

### Added

- **Multi-Source File Watcher**: Backend now monitors all local sources' `.linggen` directories recursively.
- **Incremental Indexing**: Automatic re-indexing of memories, prompts, and notes when markdown files are created, modified, or deleted.
- **Real-time UI Sync**: New SSE (Server-Sent Events) endpoint `/api/events` to push file change notifications to the frontend.
- **Dynamic Metadata**: Indexer now parses YAML frontmatter in markdown files and stores all fields as searchable metadata in LanceDB.
- **Deterministic Memory Fetching**: New `memory_fetch_by_meta` MCP tool for retrieving memories by ID or other metadata.

### Changed

- **Memory Storage**: Shifted to a filesystem-first approach. Memories are now stored as human-readable `.md` files in `.linggen/memory/`.
- **MCP Tooling**: Removed `memory_create` and `memory_update` in favor of direct file manipulation by the LLM or user.
- **Frontend Refresh**: Replaced 10-second polling with an event-driven model using `EventSource` for instantaneous UI updates.
- **Internal Indexer**: Improved robustness of the rescan process and path handling for cross-platform compatibility.

### Fixed

- Resolved compilation issues with `SourceType` equality and mismatched return types in the rescan handler.
- Fixed a bug where file removals were not correctly detected on certain operating systems (macOS) during renames.
- Corrected path ownership errors in the internal indexer.
