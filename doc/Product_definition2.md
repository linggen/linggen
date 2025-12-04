## Files Graph View (Architect v1)

### 1. Goal

- **Goal**: Provide an Obsidian-like **file dependency graph** that helps developers understand how files relate to each other (imports/usages), without needing deep AI understanding in v1.
- **Primary users**:
  - **Newcomers** to a codebase (onboarding, "what talks to what?").
  - **Maintainers** of medium/large projects (spot coupling, impact of changes).
  - **Architects / refactorers** (see modules, boundaries, potential cycles).

### 2. Scope for v1

- **Nodes**: Source files/modules (e.g. `src/main.rs`, `src/lib.rs`, `src/frontend/App.tsx`).
- **Edges**: Static "uses/imports" relations between files:
  - Rust: `use`, `mod`, `extern crate`, etc.
  - TS/JS: `import`, `require`, etc. (future).
- **Data source**: Code parsed via **Tree-sitter**:

  - Per language, define queries to extract imports.
  - Simple path resolution from module names to file paths (best-effort; can be wrong sometimes).

- **Frontend**: React web UI with Obsidian-like graph:
  - Uses a **WebGL-based graph library** (e.g. `react-force-graph`, `sigma.js`, `cytoscape.js`).
  - **Interactions**:
    - Click node → open file (in IDE or built-in viewer).
    - Hover → highlight neighbors (1-hop neighborhood).
    - Search box → jump to a file and center it.
    - Basic filters (by folder / crate / language).

### 3. Large graph UX (hundreds of files)

- **Focus + neighborhood view**:
  - Always have a **selected node**.
  - Show selected node + its **1–2 hop neighbors**; dim or hide the rest.
- **Filtering**:
  - Filter by directory/crate (`src/ingest/*`, `src/api/*`), language, degree (hide leaf nodes).
- **Clustering / grouping**:
  - Group nodes into **folders or components** as "super-nodes" that can be expanded.
- **Level of detail (LOD)**:
  - Zoomed out: only dots/clusters, no labels.
  - Zoomed in: show labels and detailed edges.

### 4. Data model and storage

- **Core graph model (conceptual)**:
  - **Node**: `{ id: "src/main.rs", label: "main.rs", language, folder, ... }`
  - **Edge**: `{ source: "src/main.rs", target: "src/lib.rs", kind: "Import" }`
- **Persistence strategy**:
  - **V1**:
    - Graph can be fully recomputed from source (Tree-sitter + filesystem).
    - Optionally cache as a **JSON file** per project.
  - **Later (using redb)**:
    - Store **user overrides** (ignored edges, manual edges).
    - Store **node/edge metadata** (tags like "service", "publisher", etc.).
    - Store **saved layouts/views** (node positions, filters).
    - Possibly store incremental analysis state for faster updates.

### 5. Integration with Linggen pipeline

- **Triggering graph build**:
  - After a project is **indexed/ingested**, start a **background task**:
    - Walk files → parse via Tree-sitter → extract imports → build graph.
    - Save or cache the graph result (in memory / JSON / redb).
  - This background task does **not block** normal query or embedding operations.
- **Serving to UI**:
  - Backend exposes a REST endpoint, e.g. `GET /projects/{id}/graph`.
  - React frontend calls this endpoint and renders the graph.

### 6. Future extensions (Architect v2+)

- **Semantic roles and topology**:
  - Tag nodes as **publishers / subscribers / services / handlers**, using:
    - Simple heuristics (e.g. functions calling `publish`, `subscribe`, HTTP clients).
    - Optional AI classification to suggest roles.
  - Build a **higher-level topology graph** (components/services and their communication).
- **AI-assisted understanding**:
  - Suggest components, topics, and flows from the existing graph + code.
  - Allow users to override and correct AI suggestions; never fully "authoritative".

### 7. Non-goals for v1

- No guaranteed **100% accurate import resolution**.
- No full **"architect-level" semantic understanding** yet (only structural file graph).
- No heavy requirement for users to configure tons of metadata before it's useful.

---

## Implementation Details

### Backend: `linggen-architect` Crate

The graph analysis is implemented in the `backend/architect` crate with the following modules:

- **`graph.rs`**: Core data structures (`FileNode`, `Edge`, `EdgeKind`, `ProjectGraph`)
- **`parser.rs`**: Tree-sitter based import extraction for Rust
- **`resolver.rs`**: Module path to file path resolution
- **`walker.rs`**: Project directory walker with ignore support
- **`cache.rs`**: JSON-based graph caching with staleness detection
- **`overrides.rs`**: User override model (hidden edges, manual edges, tags)

### API Endpoints

#### Get Graph Status

```
GET /api/sources/:source_id/graph/status
```

Response:

```json
{
  "status": "ready" | "missing" | "stale" | "building",
  "node_count": 150,
  "edge_count": 89,
  "built_at": "2024-01-15T10:30:00Z"
}
```

#### Get Graph Data

```
GET /api/sources/:source_id/graph
GET /api/sources/:source_id/graph?folder=src/handlers
GET /api/sources/:source_id/graph?focus=src/main.rs&hops=2
```

Response:

```json
{
  "project_id": "/path/to/project",
  "nodes": [
    {
      "id": "src/main.rs",
      "label": "main.rs",
      "language": "rust",
      "folder": "src"
    }
  ],
  "edges": [
    { "source": "src/main.rs", "target": "src/lib.rs", "kind": "import" }
  ],
  "built_at": "2024-01-15T10:30:00Z"
}
```

#### Rebuild Graph

```
POST /api/sources/:source_id/graph/rebuild
```

Triggers a background rebuild of the graph. Returns immediately with status "building".

### Frontend Component

The graph view is implemented in `frontend/src/components/GraphView.tsx` using `react-force-graph-2d`.

Features:

- **Search**: Find files by name
- **Filter**: Filter by folder
- **Hover**: Highlight neighbors
- **Click**: Select node and show connections
- **Zoom/Pan**: Navigate large graphs

### Usage

1. **Index a local source** in Linggen
2. **Open source details** from the Sources view
3. **View the File Dependency Graph** section
4. Use search, filters, and click interactions to explore

### Limitations

- Only Rust files are analyzed in v1 (TypeScript/JavaScript planned)
- Import resolution is best-effort and may miss some edges
- Very large projects (1000+ files) may need filtering for good performance
