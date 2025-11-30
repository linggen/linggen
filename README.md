# Linggen

**Linggen** is a local, privacy-focused RAG (Retrieval-Augmented Generation) service written in Rust. It turns your local history (git repos, docs, notes) into a searchable "second brain" for your AI tools.

## Documentation

- **[Features](doc/features.md)**: Detailed list of capabilities.
- **[Framework Architecture](doc/framework.md)**: System design and architecture diagram.
- **[Cursor MCP Setup](doc/cursor-mcp-setup.md)**: How to integrate Linggen with Cursor IDE.

## Current Status

- **Frontend**: React + Vite setup, connected to backend.
- **Backend**: Rust Axum server, CORS enabled.
- **Ingestion**: Basic file walker and watcher implemented.
- **MCP Server**: HTTP/SSE-based MCP server for Cursor integration.

## Quick Start

### Prerequisites

- Rust (latest stable)
- Node.js & npm

### Running the Project

1. **Start the Linggen Backend**:

   ```bash
   cd backend && cargo run -p api
   ```

   The API will be available at `http://localhost:3000`.

2. **Start the Frontend** (optional):

   ```bash
   cd frontend && npm run dev
   ```

   Access the web UI at `http://localhost:5173`.

3. **Start the MCP Server** (for Cursor integration):
   ```bash
   cd backend && cargo run -p mcp-http
   ```
   The MCP server will be available at `http://localhost:3001`.

### Cursor Integration

Add to your `~/.cursor/mcp.json`:

```json
{
  "mcpServers": {
    "linggen": {
      "url": "http://localhost:3001/mcp/sse"
    }
  }
}
```

See [Cursor MCP Setup](doc/cursor-mcp-setup.md) for detailed instructions.

## Architecture

```
┌─────────────┐     HTTP      ┌─────────────┐     HTTP      ┌─────────────┐
│   Cursor    │◄────────────► │  mcp-http   │◄────────────► │ Linggen API │
│   (IDE)     │    SSE        │  (Gateway)  │               │  (Backend)  │
└─────────────┘               └─────────────┘               └─────────────┘
     :3001                         :3001                         :3000
```

## License

MIT
