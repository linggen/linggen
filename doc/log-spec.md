---
type: spec
guide: |
  Product specification — describe what the system should do and why.
  Keep it brief. Aim to guide design and implementation, not document code.
  Avoid implementation details like function signatures, variable types, or code snippets.
---

# Logging Specification

## Goals

- Make sure AI coding tool like Claude code can locate and fix bug.
- Log all important steps in each service lifecycle and data path.
- Keep logs easy to read and easy to analyze in development and production.
- Prevent log storms with throttling and rate limits.

## Output Targets

- Debug console (service runtime / terminal).
- Browser console (for web-facing components and dashboards).
- File logs (persistent records for troubleshooting and audits). Current runtime rotates daily and applies retention cleanup (default 30 days).

## Required Logging Coverage

- Startup, initialization, ready, shutdown.
- Dependency wait/retry/connect/disconnect events.
- Health transitions: ready, degraded, failed, recovered.
- Restart and fail-safe events.
- Key processing milestones and major errors.

## Log Levels

- `DEBUG`: developer diagnostics and detailed state changes.
- `INFO`: normal important workflow milestones.
- `WARN`: retry loops, degraded state, non-fatal abnormal conditions.
- `ERROR`: fatal failures, panics, unrecoverable conditions.

## Readability and Format

- Use consistent, structured format (JSON lines recommended for files).
- Use concise human-readable message text.
- Include key context fields (service, module, event, correlation/request id if available).
- Color and icon usage is recommended for console output to improve scanability.

## Throttling and Rate Limiting

- Always throttle repeated high-frequency logs.
- Apply per-event throttling for noisy loops (for example retry and polling loops).
- Aggregate repeated messages into summary lines when possible.
- Protect console and file outputs from flush storms.

## Environment Behavior

- Development: allow richer `DEBUG` logs with color/icon formatting.
- Production: default to `INFO`, keep structure stable, and prioritize signal over noise.
