# Linggen

**Linggen** is a local, privacy-focused RAG (Retrieval-Augmented Generation) service written in Rust. It turns your local history (git repos, docs, notes) into a searchable "second brain" for your AI tools.

## Documentation

- **[Features](doc/features.md)**: Detailed list of capabilities.
- **[Framework Architecture](doc/framework.md)**: System design and architecture diagram.
- **[Cursor MCP Setup](doc/cursor-mcp-setup.md)**: How to integrate Linggen with Cursor IDE.

## Current Status

- **Frontend**: React + Vite setup, connected to backend.
- **Backend**: Rust Axum server with integrated MCP endpoint.
- **Desktop App**: Tauri 2.9-based native app for macOS (bundles backend as sidecar).
- **Ingestion**: Basic file walker and watcher implemented.
- **MCP**: Built-in SSE endpoint for Cursor integration (no separate binary needed).

## Quick Start

### Prerequisites

- Rust (latest stable, 1.70+)
- Node.js & npm (for frontend and Tauri CLI)
- macOS 12.0+ (for desktop app)

### Option 1: Desktop App (Recommended)

Build and run the native Tauri desktop app:

```bash
./build-tauri-app.sh
open frontend/src-tauri/target/release/bundle/macos/Linggen.app
```

### Option 2: Development Mode

1. **Start the Backend Server**:

   ```bash
   cd backend && cargo run -p api --release
   ```

   This starts:

   - **API** at `http://localhost:8787/api/*`
   - **MCP endpoint** at `http://localhost:8787/mcp/*`
   - **Frontend** at `http://localhost:8787/` (if built)

2. **Start the Frontend** (for development with hot reload):

   ```bash
   cd frontend && npm run dev
   ```

   Access the web UI at `http://localhost:5173`.

3. **Or run Tauri in dev mode** (native window + hot reload):

   ```bash
   # Terminal 1: Start backend
   cd backend && cargo run -p api --release

   # Terminal 2: Start Tauri dev
   cd frontend && npm run tauri:dev
   ```

   > **Note**: When running `tauri:dev`, if the backend is already running (e.g., from Terminal 1 or your IDE), the Tauri app will detect it and connect to it instead of spawning its own sidecar. This allows seamless debugging of backend changes while using the native desktop window.

### Remote Access

The backend listens on all interfaces, so remote users on your network can access the UI:

```
http://<your-ip>:8787
```

### Cursor Integration

Add to your `~/.cursor/mcp.json`:

```json
{
  "mcpServers": {
    "linggen": {
      "url": "http://localhost:8787/mcp/sse"
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
│                                                                 │
│  ┌─────────┐   Sidecar    ┌──────────────────────────────────┐  │
│  │ Tauri   │◄────────────►│  Native desktop window           │  │
│  │ Desktop │              │  (embeds backend + frontend)     │  │
│  └─────────┘              └──────────────────────────────────┘  │
└─────────────────────────────────────────────────────────────────┘
                          localhost:8787
```

## License

MCP publish:
https://modelcontextprotocol.info/tools/registry/cli/
