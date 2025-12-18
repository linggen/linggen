# Linggen Extension Webview Embed

This document describes how a VS Code/Cursor extension can embed Linggen’s UI as a webview using a URL like `http://localhost:8787/extension`.

## URL scheme

The embedded page is served by the Linggen backend (same origin as the API).

Example:

- `http://127.0.0.1:8787/extension?tab=explain&source_id=<sourceId>&file_path=src/main.rs&selection=<urlEncoded>`

### Query parameters

- **tab**: `explain | graph | prompts` (optional, default: `explain`)
- **source_id**: Linggen source id (required)
- **file_path**: path relative to the source root (optional)
- **selection**: URL-encoded editor selection (optional)
- **symbol**: symbol under cursor (optional)

## Recommended extension flow (stateless)

### 1) Register a context menu command

- Add a command like `linggen.explainAcrossProjects` and contribute it to `editor/context`.

### 2) On command execution

- Determine `source_id` for current workspace:
  - call `GET /api/resources`
  - choose the resource whose `path` is the longest prefix of the workspace folder
  - cache mapping in extension settings
- Collect editor context: `file_path`, `selection`, `cursor position`, optional `symbol`.

### 3) Show or create the webview panel

- Create a single bottom panel webview and reuse it.

### 4) Deep link or postMessage

Two options:

- **Deep link (simple)**: set the webview to `http://127.0.0.1:8787/extension?...` (reloads the page)
- **postMessage (recommended)**: keep the page loaded once and send context updates:
  - `{ type: "explainAcrossProjects", payload: { source_id, file_path, selection, symbol } }`

## Linggen endpoints used by the embedded page

- **Explain Across Projects**: `POST /api/query` with `{ query, limit, exclude_source_id }`
- **Graph (focused)**: `GET /api/sources/:source_id/graph/focus?file_path=...&hops=1`
- **Prompt templates**:
  - `GET /api/sources/:source_id/prompts`
  - `GET/PUT/DELETE /api/sources/:source_id/prompts/*file_path`
  - `POST /api/sources/:source_id/prompts/rename`
