# CLI Integration Refactoring Plan

## Status: IN PROGRESS

This document tracks the refactoring to integrate the CLI into the backend binary.

## Completed ✅

1. **Moved CLI code to backend/api/src/cli/**

   - ✅ Copied commands.rs → cli/commands.rs
   - ✅ Copied api.rs → cli/client.rs
   - ✅ Created cli/mod.rs
   - ✅ Updated imports in commands.rs

2. **Updated Cargo.toml**
   - ✅ Added clap, colored, tabled dependencies to backend/api

## In Progress 🚧

3. **Refactor main.rs** - NEXT STEP
   - Need to add clap parsing for subcommands
   - Extract server initialization into `serve()` function
   - Add CLI command routing
   - Implement hybrid start command with auto-launch

## Remaining Tasks 📋

4. **Implement Auto-Start**

   - Add `spawn_server_daemon()` function
   - Add `ensure_backend_running()` helper
   - Implement smart `start` command

5. **Update Workspace**

   - Remove `backend/cli` from workspace members
   - Update binary name references

6. **Update Build Scripts**

   - Modify `deploy/build-tauri-app.sh`
   - Update binary names (linggen instead of api)
   - Update sidecar configuration

7. **Update Tauri Config**

   - Change external bin from `linggen-backend` and `linggen-cli` to just `linggen`
   - Update sidecar references

8. **Update VS Code Configs**

   - Update launch.json configurations
   - Update tasks.json
   - Update paths and binary names

9. **Update Documentation**
   - README.md
   - CLI_BUNDLING.md
   - IMPLEMENTATION_SUMMARY.md
   - Installation scripts

## New Binary Structure

```
# Before
backend/target/release/api       # Server
backend/target/release/linggen   # CLI

# After
backend/target/release/linggen   # Both server and CLI

# Usage
linggen serve                    # Start server (default if no args)
linggen start                    # Check/start backend
linggen index <path>             # Index via CLI
linggen status                   # Show status
```

## Main.rs Structure (Planned)

```rust
#[derive(Parser)]
struct Cli {
    #[command(subcommand)]
    command: Option<Commands>,

    /// API URL for CLI commands
    #[arg(long, env = "LINGGEN_API_URL", default_value = "http://127.0.0.1:8787")]
    api_url: String,
}

#[derive(Subcommand)]
enum Commands {
    /// Start the Linggen server
    Serve {
        #[arg(short, long, default_value = "8787")]
        port: u16,

        #[arg(short, long)]
        daemon: bool,
    },

    /// Check backend status and optionally start it
    Start {
        #[arg(long)]
        auto_start: bool,
    },

    /// Index a directory
    Index { /* ... */ },

    /// Show status
    Status { /* ... */ },
}

#[tokio::main]
async fn main() {
    let cli = Cli::parse();

    match cli.command {
        None | Some(Commands::Serve { .. }) => {
            // Server mode
            serve(port, daemon).await
        }
        Some(Commands::Start { auto_start }) => {
            // Check/start backend
            handle_start_with_auto_launch(api_url, auto_start).await
        }
        Some(Commands::Index { .. }) => {
            // CLI mode
            let client = ApiClient::new(cli.api_url);
            cli::handle_index(&client, ...).await
        }
        // ... other commands
    }
}
```

## Key Design Decisions

1. **Binary Name**: `linggen` (not `api`)
2. **Default Behavior**: `linggen` with no args starts server
3. **Explicit Subcommands**: `linggen serve`, `linggen start`, etc.
4. **Auto-Start**: `linggen start --auto-start` or via `linggen start` with prompt
5. **Backward Compat**: Keep `--port` and other server flags

## Breaking Changes

- Binary name changes from `api` to `linggen`
- Need to update all references in:
  - Tauri config
  - Build scripts
  - VS Code configs
  - Documentation
  - Installation scripts

## Testing Checklist

- [ ] `linggen` starts server
- [ ] `linggen serve` starts server
- [ ] `linggen start` checks status
- [ ] `linggen start --auto-start` starts if not running
- [ ] `linggen index /path` works
- [ ] `linggen status` works
- [ ] Tauri app bundles correctly
- [ ] DMG installs with CLI working
- [ ] VS Code launch configs work

## Rollback Plan

If issues arise, we can:

1. Keep both binaries temporarily
2. Revert workspace changes
3. Maintain separate CLI crate alongside

## Next Steps

1. Complete main.rs refactoring
2. Test basic server start
3. Test CLI commands
4. Update build system
5. Update documentation
6. Full integration test

---

**Current Status**: Step 3 in progress - refactoring main.rs
**Last Updated**: December 8, 2024
