# Linggen

**Linggen** is a local, privacy-focused RAG (Retrieval-Augmented Generation) service written in Rust. It turns your local history (git repos, docs, notes) into a searchable "second brain" for your AI tools.

## Documentation

- **[Features](doc/features.md)**: Detailed list of capabilities.
- **[Framework Architecture](doc/framework.md)**: System design and architecture diagram.
- **[Cursor MCP Setup](doc/cursor-mcp-setup.md)**: How to integrate Linggen with Cursor IDE.

## Current Status

- **Frontend**: React + Vite setup, connected to backend.
- **Backend**: Rust Axum server with integrated MCP endpoint.
- **Ingestion**: Basic file walker and watcher implemented.
- **MCP**: Built-in SSE endpoint for Cursor integration (no separate binary needed).

## Quick Start

### Prerequisites

- Rust (latest stable)
- Node.js & npm (for frontend development)

### Running the Project

1. **Start the Linggen Server**:

   ```bash
   cd backend && cargo run -p api --release
   ```

   This starts:
   - **API** at `http://localhost:7000/api/*`
   - **MCP endpoint** at `http://localhost:7000/mcp/*`
   - **Frontend** at `http://localhost:7000/` (if built)

2. **Start the Frontend** (for development):

   ```bash
   cd frontend && npm run dev
   ```

   Access the web UI at `http://localhost:5173`.

### Cursor Integration

Add to your `~/.cursor/mcp.json`:

```json
{
  "mcpServers": {
    "linggen": {
      "url": "http://localhost:7000/mcp/sse"
    }
  }
}
```

Restart Cursor, and the Linggen tools will be available in chat.

See [Cursor MCP Setup](doc/cursor-mcp-setup.md) for detailed instructions and team setup.

## Architecture

```
┌─────────────────────────────────────────────────────────────────┐
│                         Linggen Server                          │
│                                                                 │
│  ┌─────────┐     SSE      ┌──────────────────────────────────┐  │
│  │ Cursor  │◄────────────►│  /mcp/*  - MCP SSE endpoint      │  │
│  └─────────┘              │  /api/*  - REST API              │  │
│                           │  /       - Frontend (if built)   │  │
│  ┌─────────┐     HTTP     └──────────────────────────────────┘  │
│  │ Web UI  │◄────────────►                                      │
│  └─────────┘                                                    │
└─────────────────────────────────────────────────────────────────┘
                          localhost:7000
```

## License

MIT
