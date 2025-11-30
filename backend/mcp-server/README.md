# Linggen MCP Server (stdio) - DEPRECATED

> **⚠️ DEPRECATED:** This stdio-based MCP server requires installing a local binary on each user's machine.
> 
> **Use the unified Linggen API server instead**, which includes MCP endpoints at `/mcp/*`.
> See [Cursor MCP Setup Guide](../../doc/cursor-mcp-setup.md) for instructions.

## Recommended: Use Unified API Server

The Linggen API server now includes MCP endpoints. No separate binary needed!

1. Start the server:
   ```bash
   cargo run -p api --release
   ```

2. Configure Cursor (`~/.cursor/mcp.json`):
   ```json
   {
     "mcpServers": {
       "linggen": {
         "url": "http://localhost:7000/mcp/sse"
       }
     }
   }
   ```

For team setups, just point to your shared server:
```json
{
  "mcpServers": {
    "linggen": {
      "url": "http://linggen.company.internal:7000/mcp/sse"
    }
  }
}
```

**No local installation required for team members!**

---

## Legacy: stdio MCP Server (Not Recommended)

If you still need the stdio-based server for some reason:

### Build

```bash
cargo build -p mcp-server --release
```

### Configure Cursor

```json
{
  "mcpServers": {
    "linggen": {
      "command": "/path/to/mcp-server",
      "args": [],
      "env": {
        "LINGGEN_API_URL": "http://localhost:7000"
      }
    }
  }
}
```

### Debug

```bash
tail -100f /tmp/linggen-mcp.log
```
