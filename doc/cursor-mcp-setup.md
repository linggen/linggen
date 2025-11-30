# Cursor MCP Setup for Linggen

This guide explains how to configure Cursor to use Linggen's MCP tools for code search and prompt enhancement.

## Quick Setup (Recommended)

### 1. Global Configuration

Add Linggen to your global Cursor MCP configuration so it's available in all projects.

Create or edit `~/.cursor/mcp.json`:

```json
{
  "mcpServers": {
    "linggen": {
      "url": "http://localhost:3001/mcp/sse"
    }
  }
}
```

### 2. Restart Cursor

After saving the configuration, restart Cursor to load the new MCP server.

### 3. Verify Connection

1. Open Cursor Settings (`Cmd+Shift+J` on Mac, `Ctrl+Shift+J` on Windows/Linux)
2. Go to **Features > Model Context Protocol**
3. You should see "linggen" listed as an available server
4. The tools should appear under "Available Tools" in chat

## Team/LAN Setup

For teams sharing a central Linggen server:

```json
{
  "mcpServers": {
    "linggen": {
      "url": "http://linggen.your-company.internal:3001/mcp/sse"
    }
  }
}
```

Replace `linggen.your-company.internal` with your actual server hostname or IP.

## Project-Specific Configuration

If you want Linggen only for specific projects, create `.cursor/mcp.json` in the project root:

```json
{
  "mcpServers": {
    "linggen": {
      "url": "http://localhost:3001/mcp/sse"
    }
  }
}
```

This configuration will override global settings for that project.

## Available Tools

Once connected, these tools are available in Cursor chat:

| Tool              | Description                       | Example Usage                            |
| ----------------- | --------------------------------- | ---------------------------------------- |
| `search_codebase` | Search for relevant code snippets | "Search for authentication code"         |
| `enhance_prompt`  | Enhance your prompt with context  | "Enhance: How does the user login work?" |
| `list_sources`    | List indexed codebases            | "What sources are indexed?"              |
| `get_status`      | Check Linggen server status       | "What's the Linggen status?"             |

## Troubleshooting

### Server not appearing in Cursor

1. Check that the MCP HTTP server is running:

   ```bash
   curl http://localhost:3001/health
   ```

   Should return `OK`.

2. Verify your `mcp.json` syntax is valid JSON.

3. Restart Cursor completely (not just reload window).

### Tools not working

1. Check that the Linggen backend API is running:

   ```bash
   curl http://localhost:3000/api/status
   ```

2. Check MCP server logs for errors.

3. Verify `LINGGEN_API_URL` is set correctly for the MCP HTTP server.

### View MCP Logs

In Cursor:

1. Open the Output panel (`Cmd+Shift+U`)
2. Select "MCP Logs" from the dropdown
3. Look for connection errors or tool call failures

## Running the MCP HTTP Server

### Development

```bash
cd backend
cargo run -p mcp-http
```

### Production

```bash
cd backend
cargo build -p mcp-http --release
./target/release/mcp-http
```

### Environment Variables

| Variable          | Default                 | Description             |
| ----------------- | ----------------------- | ----------------------- |
| `LINGGEN_API_URL` | `http://localhost:3000` | Linggen backend API URL |
| `MCP_HTTP_PORT`   | `3001`                  | MCP HTTP server port    |

## Architecture Overview

```
┌─────────────────────────────────────────────────────────────────┐
│                         Your Machine                            │
│  ┌─────────┐                                                    │
│  │ Cursor  │◄──── SSE ────┐                                     │
│  └─────────┘              │                                     │
│                           ▼                                     │
│                    ┌─────────────┐      HTTP      ┌───────────┐ │
│                    │  mcp-http   │◄─────────────► │ Linggen   │ │
│                    │  (Gateway)  │                │ API       │ │
│                    └─────────────┘                └───────────┘ │
│                    localhost:3001                localhost:3000 │
└─────────────────────────────────────────────────────────────────┘
```

For team setups, the MCP HTTP server and Linggen API run on a shared server, and each developer's Cursor connects via the network.
