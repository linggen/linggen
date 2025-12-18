# Linggen Internal Index

## Overview

Linggen uses a separate LanceDB table (`internal_chunks`) to index its own internal content (memories and prompt templates) separately from project/source code. This separation provides several benefits:

- **Clean separation**: Internal content doesn't pollute the main code index
- **Incremental updates**: Changes to memories/prompts are indexed immediately without requiring a full source reindex
- **Selective search**: APIs can choose to include or exclude internal content from search results

## Architecture

### Storage

- **Main index**: `chunks_main` table - contains code chunks from indexed projects/sources
- **Internal index**: `internal_chunks` table - contains chunks from `.linggen/memory/` and `.linggen/prompts/` files

Both tables use the same schema and share the same LanceDB database directory.

### Chunk Metadata

Internal chunks are tagged with:
- `source_id`: Prefixed with `linggen-internal:` to distinguish from project sources
- `document_id`: Relative path within `.linggen/` (e.g., `memory/20250117-143022__auth-flow__a1b2c3d4.md`)
- `metadata.kind`: Either `"memory"` or `"prompt"` to categorize the content type

## Automatic Indexing

### When Files Are Created/Updated

When you save a memory or prompt file through the API:

1. **Memory files** (`POST /api/sources/:source_id/memory/*file_path`):
   - File is written to `.linggen/memory/`
   - Automatically indexed in the background to `internal_chunks`
   - Old chunks for that file are removed, new chunks are added

2. **Prompt files** (`PUT /api/sources/:source_id/prompts/*file_path`):
   - File is written to `.linggen/prompts/`
   - Automatically indexed in the background to `internal_chunks`
   - Old chunks for that file are removed, new chunks are added

### When Files Are Deleted

When you delete a memory or prompt file:
- The file is removed from disk
- All associated chunks are automatically removed from `internal_chunks`

## Manual Rescan

If you edit memory or prompt files outside of the Linggen API (e.g., directly in a text editor), you can trigger a manual rescan to update the internal index:

```bash
POST /api/sources/:source_id/internal/rescan
```

**Response:**
```json
{
  "files_indexed": 15,
  "files_failed": 0
}
```

This endpoint:
- Recursively scans `.linggen/memory/` and `.linggen/prompts/`
- Re-indexes all `.md` files
- Removes old chunks and adds new ones for each file

## Search Integration

### REST API

The `/api/search` endpoint supports an `include_internal` flag:

```bash
POST /api/search
Content-Type: application/json

{
  "query": "authentication flow",
  "limit": 10,
  "include_internal": true
}
```

- `include_internal: false` (default): Only search main code index
- `include_internal: true`: Search both main and internal indexes

### MCP Tools

The MCP tools (`search_codebase`, `enhance_prompt`, `query_codebase`) currently search only the main index by default. To include internal content, you would need to extend the tool parameters (future enhancement).

## Implementation Details

### Embedding Model

Internal chunks use the same embedding model as the main index. If the embedding model is not available:
- Chunks are still created and stored
- Embeddings are set to `None`
- Search will fall back to keyword matching

### Chunking

Internal files are chunked using the same `TextChunker` as code files, which:
- Splits on paragraph boundaries
- Respects markdown structure
- Aims for ~500-1000 token chunks

### Incremental Updates

The `upsert_document` function ensures incremental updates:

```rust
pub async fn upsert_document(&self, document_id: &str, chunks: Vec<Chunk>) -> Result<()>
```

1. Deletes all existing chunks with matching `document_id`
2. Adds new chunks
3. This ensures no stale chunks remain when content changes

## Best Practices

1. **Use the API**: Prefer using the Linggen API to create/update/delete memories and prompts, as this ensures automatic indexing.

2. **Rescan after bulk edits**: If you make many manual edits to `.linggen/` files, run a rescan to update the index.

3. **Monitor rescan results**: Check the `files_failed` count in rescan responses. Non-zero values indicate files that couldn't be indexed (e.g., invalid markdown, encoding issues).

4. **Search with context**: When searching for memories/prompts, use `include_internal: true`. When searching for code only, omit the flag or set it to `false`.

## Troubleshooting

### Memory/prompt not appearing in search

1. Check if the file exists in `.linggen/memory/` or `.linggen/prompts/`
2. Trigger a manual rescan: `POST /api/sources/:source_id/internal/rescan`
3. Check the rescan response for `files_failed`

### Stale search results

If you edited a file manually and search still shows old content:
- Run a rescan for that source
- The `upsert_document` logic will replace old chunks with new ones

### Performance

The internal index is typically small (dozens to hundreds of files), so rescans are fast (< 1 second). If you have thousands of memory/prompt files, consider:
- Organizing into subdirectories
- Archiving old memories
- Using more selective searches

## Future Enhancements

- **MCP integration**: Add `include_internal` parameter to MCP tools
- **Selective rescan**: Rescan only changed files (using file timestamps)
- **Internal-only search**: Add a dedicated endpoint for searching only internal content
- **Cross-source memories**: Support memories that span multiple projects

