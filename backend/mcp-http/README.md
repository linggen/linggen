# Linggen MCP HTTP/SSE Server

Remote MCP server for Linggen that allows Cursor users to connect via URL without installing a local binary.

## Overview

This server exposes MCP tools over HTTP/SSE:

- `GET /mcp/sse` - Server-Sent Events endpoint for server-to-client streaming
- `POST /mcp/message` - HTTP endpoint for client-to-server MCP messages
- `GET /health` - Health check endpoint

## Build

```bash
cargo build -p mcp-http --release
```

The binary will be at `backend/target/release/mcp-http`.

## Run

```bash
# Set the Linggen API URL (defaults to http://localhost:3000)
export LINGGEN_API_URL="http://localhost:3000"

# Optionally set the port (defaults to 3001)
export MCP_HTTP_PORT=3001

# Run the server
./target/release/mcp-http
```

## Cursor Configuration

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

For team/LAN deployment:

```json
{
  "mcpServers": {
    "linggen": {
      "url": "http://linggen.company.internal:3001/mcp/sse"
    }
  }
}
```

## Available Tools

| Tool              | Description                                                  |
| ----------------- | ------------------------------------------------------------ |
| `search_codebase` | Search the Linggen knowledge base for relevant code snippets |
| `enhance_prompt`  | Enhance a user prompt with relevant context                  |
| `list_sources`    | List all indexed sources/projects                            |
| `get_status`      | Get the current status of the Linggen backend                |

## Architecture

```
┌─────────────┐     SSE/HTTP      ┌──────────────┐      HTTP       ┌─────────────┐
│   Cursor    │ ◄───────────────► │  mcp-http    │ ◄─────────────► │ Linggen API │
│  (Client)   │                   │  (Gateway)   │                 │  (Backend)  │
└─────────────┘                   └──────────────┘                 └─────────────┘
```

- **Cursor** connects to `mcp-http` via SSE for streaming responses
- **mcp-http** translates MCP requests into Linggen API calls
- **Linggen API** provides the actual RAG functionality

## Environment Variables

| Variable               | Default                 | Description                                    |
| ---------------------- | ----------------------- | ---------------------------------------------- |
| `LINGGEN_API_URL`      | `http://localhost:3000` | URL of the Linggen backend API                 |
| `MCP_HTTP_PORT`        | `3001`                  | Port for the MCP HTTP server                   |
| `LINGGEN_ACCESS_TOKEN` | (none)                  | Optional access token for basic authentication |

## Access Control

For LAN deployments where you want basic access control, set `LINGGEN_ACCESS_TOKEN`:

```bash
export LINGGEN_ACCESS_TOKEN="your-secret-token"
./target/release/mcp-http
```

Then configure Cursor with the token:

```json
{
  "mcpServers": {
    "linggen": {
      "url": "http://linggen.company.internal:3001/mcp/sse",
      "headers": {
        "X-Linggen-Token": "your-secret-token"
      }
    }
  }
}
```

Alternatively, use the `Authorization` header:

```json
{
  "mcpServers": {
    "linggen": {
      "url": "http://linggen.company.internal:3001/mcp/sse",
      "headers": {
        "Authorization": "Bearer your-secret-token"
      }
    }
  }
}
```

## Health Check

The `/health` endpoint returns server stats:

```bash
curl http://localhost:3001/health
# {"status":"ok","active_connections":2,"total_requests":15}
```
