---
id: rust-rules
name: Rust Conventions
description: Idiomatic Rust patterns, safety rules, and strict testing policies.
scope: Curated
version: 1.2.0
author: Linggen
tags: ["rust", "safety", "testing"]
---

# Rust Conventions

- Use `anyhow` for application-level error handling.
- Prefer `clippy` for linting; all clippy warnings should be addressed.
- Use `tokio` for async runtimes.
- Avoid `unsafe` unless absolutely necessary and documented.
- Follow the official Rust Style Guide for naming and formatting.
- When add new dependence, pleaes ask for confirm and make sure use latest version
- Run cargo check after change code
- Avoid long single function split into shorter one, make sure code is always readable and maintainable 