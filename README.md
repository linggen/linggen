# Linggen

**Linggen** is a local, privacy-focused RAG (Retrieval-Augmented Generation) service written in Rust. It turns your local history (git repos, docs, notes) into a searchable "second brain" for your AI tools.

## Documentation

- **[Features](doc/features.md)**: Detailed list of capabilities.
- **[Framework Architecture](doc/framework.md)**: System design and architecture diagram.
- **[Cursor MCP Setup](doc/cursor-mcp-setup.md)**: How to integrate Linggen with Cursor IDE.

## CLI Tool

Linggen provides a unified binary that acts as both the server and CLI tool:

### Installation

#### Option 1: From DMG/App Bundle (Recommended for macOS)

If you've installed Linggen.app from the DMG, the binary is already bundled:

```bash
# Run the installation helper script
./install-cli-from-app.sh
```

This creates a symlink from the bundled binary to `/usr/local/bin/linggen`.

#### Option 2: Build from Source

Build from source:

```bash
cd backend
cargo build --release --bin linggen
```

Add the binary to your PATH:

```bash
# macOS/Linux
cp target/release/linggen /usr/local/bin/

# Or add the target/release directory to your PATH
export PATH="$PWD/target/release:$PATH"
```

Alternatively, use the installation script:

```bash
./install-cli.sh
```

### Usage

#### Start Server

```bash
# Start the server (foreground)
linggen serve
# Or just:
linggen

# With custom port
linggen serve --port 9000
```

#### Check Backend Status

```bash
linggen start
```

Checks if the Linggen backend is running and displays its current status.

#### Index a Directory

```bash
# Auto mode (default) - automatically chooses incremental or full
# Uses incremental if source was previously indexed, full otherwise
linggen index /path/to/your/project

# Explicit incremental indexing
linggen index /path/to/your/project --mode incremental

# Full reindex (rebuild from scratch)
linggen index /path/to/your/project --mode full

# Index with custom name
linggen index /path/to/your/project --name "My Project"

# Index with file patterns
linggen index /path/to/your/project --include "*.rs" --include "*.md" --exclude "target/**"

# Wait for indexing to complete
linggen index /path/to/your/project --wait
```

#### Check Status and View Jobs

```bash
linggen status

# Show more recent jobs
linggen status --limit 20
```

### Configuration

The CLI can be configured via command-line flags or environment variables:

- `--api-url <URL>` or `LINGGEN_API_URL`: Base URL of the Linggen backend (default: `http://127.0.0.1:8787`)

Example:

```bash
export LINGGEN_API_URL=http://localhost:8787
linggen status
```

## VS Code Extension

The Linggen VS Code extension provides seamless integration with your editor.

### Installation

1. Open VS Code
2. Navigate to the `vscode-extension` directory
3. Install dependencies: `npm install`
4. Press F5 to launch the extension in development mode

For production use, package the extension:

```bash
cd vscode-extension
npm install -g @vscode/vsce
vsce package
code --install-extension linggen-0.1.0.vsix
```

### Commands

Open the Command Palette (Cmd+Shift+P / Ctrl+Shift+P) and type "Linggen":

- **Linggen: Index Current Workspace** - Incrementally index your workspace
- **Linggen: Full Reindex Current Workspace** - Perform a full reindex
- **Linggen: Check Backend Status** - Check if the backend is running
- **Linggen: Open in Linggen** - Open Linggen in your browser

### Settings

Configure the extension in VS Code settings:

- `linggen.cliPath`: Path to the linggen CLI binary (default: "linggen")
- `linggen.indexModeDefault`: Default indexing mode - "incremental" or "full" (default: "incremental")
- `linggen.apiUrl`: Base URL of the Linggen backend API (default: "http://127.0.0.1:8787")

### Requirements

The VS Code extension requires the Linggen CLI to be installed and available on your PATH (see CLI installation above).

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
   cd backend && cargo run --bin linggen --release
   # Or explicitly:
   cd backend && cargo run --bin linggen --release -- serve
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
   cd backend && cargo run --bin linggen --release

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
