
## Build

cargo build -p mcp-server --release
chmod 777 /Users/lianghuang/workspace/rust/rememberme/backend/target/release/mcp-server

## In Cursor mcp.json

{
  "mcpServers": {
    "rememberme": {
      "command": "/Users/lianghuang/workspace/rust/rememberme/backend/target/release/mcp-server",
      "args": [],
      "env": {
        "REMEMBERME_API_URL": "http://localhost:3000"
      }
    }
  }
}


## Debug
tail -100f  /tmp/rememberme-mcp.log