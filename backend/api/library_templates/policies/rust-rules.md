---
name: Rust Conventions
description: Idiomatic Rust patterns, safety rules, and strict verification policies.
scope: Curated
version: 1.3.0
author: Linggen
tags: ["rust", "safety", "idioms"]
---

# Rust Conventions

## Core Principles

- **Safety First**: Avoid `unsafe` code unless absolutely necessary. If used, it must be accompanied by a `// SAFETY:` comment explaining why it is sound.
- **Idiomatic Naming**: Strictly follow [RFC 430](https://rust-lang.github.io/api-guidelines/naming.html) (snake_case for functions/variables, PascalCase for types/traits).
- **Maintainability**: Avoid deeply nested logic. Favor early returns and split large functions into smaller, testable units.

## Error Handling

- **Application Level**: Use `anyhow` for high-level application logic and binary entry points.
- **Library Level**: Prefer `thiserror` for defining domain-specific error types in shared modules.
- **Propagation**: Use the `?` operator instead of `unwrap()` or `expect()`, except in test code or where a panic is mathematically impossible.

## Dependencies

- **Latest Versions**: When adding new dependencies, always use the latest stable version.
- **Confirmation**: Proactively ask the user for confirmation before adding any new crate to `Cargo.toml`.
- **Minimalism**: Be critical of "heavy" dependencies; prefer standard library solutions or lightweight alternatives when possible.

## Async and Concurrency

- **Runtime**: Use `tokio` for async operations.
- **Blocking**: Never perform heavy CPU-bound or blocking I/O operations directly inside an async task; use `tokio::task::spawn_blocking` instead.

## Verification & Tooling

- **Check**: Always run `cargo check` after code changes to verify compilation.
- **Clippy**: Prefer `clippy` for linting. All code should be "clippy-clean"; address all warnings before finalizing a task.
- **Formatting**: Ensure code follows `rustfmt` standards.
