# Internal Index Implementation Status

## Completed ✅

### 1. Internal Index Table
- **File**: `backend/storage/src/internal_index.rs`
- Created `InternalIndexStore` with LanceDB table `internal_chunks`
- Supports upsert/delete operations
- Handles incremental updates (removes old chunks, adds new ones)

### 2. Automatic Indexing Hooks
- **Memory files**: `backend/api/src/handlers/source_memory.rs`
  - Automatically indexes on save
  - Automatically removes on delete
- **Prompt files**: `backend/api/src/handlers/prompts.rs`
  - Automatically indexes on save
  - Automatically removes on delete
- Background async indexing to avoid blocking API responses

### 3. Search Integration
- **File**: `backend/api/src/handlers/search.rs`
- Added `include_internal` flag to `SearchRequest`
- Searches both main and internal indexes when flag is true
- Results are merged and filtered appropriately

### 4. Core Indexing Logic
- **File**: `backend/api/src/internal_indexer.rs`
- `index_internal_file`: Indexes a single memory/prompt file
- `remove_internal_file`: Removes chunks for a deleted file
- `rescan_internal_files`: Rescans all internal files for a source
- Handles embedding generation, chunking, and metadata

### 5. Documentation
- **File**: `doc/internal-index.md`
- Comprehensive documentation of architecture
- Usage examples for search API
- Troubleshooting guide

## Known Issues ⚠️

### Manual Rescan Endpoint
**Status**: Handler function implemented but cannot be registered as route

**Issue**: The `rescan_internal_index` function in `backend/api/src/handlers/internal_rescan.rs` triggers an Axum `Handler` trait error when trying to register it as a route.

**Error**:
```
error[E0277]: the trait bound `fn(State<Arc<...>>, ...) -> ... {rescan_internal_index}: Handler<_, _>` is not satisfied
```

**Impact**: Minor - the automatic indexing on save/delete is the primary mechanism. Manual rescan would only be needed for out-of-band file edits.

**Workaround**: Users can trigger indexing by re-saving files through the API.

**Next Steps**: Investigate Axum version compatibility or handler signature requirements. The function signature appears identical to other working handlers, suggesting a subtle type system or async trait issue.

## Testing Checklist

- [x] Internal index table creation
- [x] Memory file save triggers indexing
- [x] Memory file delete removes chunks
- [x] Prompt file save triggers indexing
- [x] Prompt file delete removes chunks
- [x] Search with `include_internal: false` (default)
- [x] Search with `include_internal: true`
- [ ] Manual rescan endpoint (blocked by handler issue)

## Usage Example

```bash
# Create a memory file (automatically indexed)
POST /api/sources/my-source/memory/test.md
Content-Type: application/json
{
  "content": "# Authentication Flow\n\nOur app uses JWT tokens..."
}

# Search including internal content
POST /api/search
Content-Type: application/json
{
  "query": "authentication",
  "limit": 10,
  "include_internal": true
}

# Response includes chunks from both code and memories
{
  "results": [
    {
      "source_id": "my-source",
      "document_id": "src/auth.rs",
      "content": "fn authenticate_user() { ... }",
      ...
    },
    {
      "source_id": "linggen-internal:my-source",
      "document_id": "memory/test.md",
      "content": "# Authentication Flow\n\nOur app uses JWT tokens...",
      ...
    }
  ]
}
```

## Architecture Summary

```
┌─────────────────────────────────────────────────┐
│  LanceDB Database                               │
├─────────────────────────────────────────────────┤
│                                                 │
│  ┌──────────────────┐  ┌──────────────────┐   │
│  │  chunks_main     │  │ internal_chunks  │   │
│  │  (project code)  │  │ (memories/prompts)│   │
│  └──────────────────┘  └──────────────────┘   │
│         ▲                      ▲                │
│         │                      │                │
└─────────┼──────────────────────┼────────────────┘
          │                      │
     ┌────┴────┐           ┌─────┴──────┐
     │ Ingestor│           │  Internal  │
     │         │           │  Indexer   │
     └─────────┘           └────────────┘
          ▲                      ▲
          │                      │
     ┌────┴────────┐       ┌─────┴────────────┐
     │ Index Source│       │ Memory/Prompt    │
     │ API         │       │ Save/Delete APIs │
     └─────────────┘       └──────────────────┘
```

## Performance Notes

- Internal index is typically small (dozens to hundreds of files)
- Automatic indexing runs in background (non-blocking)
- Search with `include_internal: true` adds minimal overhead
- Embedding generation uses the same model as main index

