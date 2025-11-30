# Linggen MCP Server (stdio) - DEPRECATED

> **Note:** This stdio-based MCP server requires installing a local binary on each user's machine.
> For team environments, use the new **HTTP/SSE MCP server** (`mcp-http`) instead.
> See [mcp-http/README.md](../mcp-http/README.md) for setup instructions.

## Build

```bash
cargo build -p mcp-server --release
chmod +x /Users/lianghuang/workspace/rust/linggen/backend/target/release/mcp-server
```

## In Cursor mcp.json (Local Binary)

```json
{
  "mcpServers": {
    "linggen": {
      "command": "/Users/lianghuang/workspace/rust/linggen/backend/target/release/mcp-server",
      "args": [],
      "env": {
        "LINGGEN_API_URL": "http://localhost:3000"
      }
    }
  }
}
```

## Recommended: Use HTTP/SSE MCP Server Instead

For team setups where you don't want each user to install a binary:

```json
{
  "mcpServers": {
    "linggen": {
      "url": "http://linggen.company.internal:3001/mcp/sse"
    }
  }
}
```

See [mcp-http/README.md](../mcp-http/README.md) for full documentation.

## Debug

```bash
tail -100f /tmp/linggen-mcp.log
```
