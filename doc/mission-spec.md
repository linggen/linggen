---
type: spec
guide: |
  Product specification — describe what the system should do and why.
  Keep it brief. Aim to guide design and implementation, not document code.
  Avoid implementation details like function signatures, variable types, or code snippets.
---

# Mission System

Cron-based scheduled agent work. A project can have **multiple active missions** — like a crontab with multiple entries.

## Related docs

- `agents.md`: agent types, lifecycle, delegation.
- `product-spec.md`: mission system overview, OS analogy.
- `storage.md`: mission JSON format, filesystem layout.

## Core concepts

A **mission** is a cron job for an agent:

| Field | Required | Description |
|:------|:---------|:------------|
| `id` | yes | Unique identifier (generated) |
| `schedule` | yes | Cron expression (5-field standard) |
| `agent_id` | yes | Which agent runs the prompt |
| `prompt` | yes | The instruction sent to the agent |
| `model` | no | Model override for this mission |
| `enabled` | yes | Whether this mission is active |
| `created_at` | yes | Timestamp |

### Cron syntax

Standard 5-field cron: `minute hour day-of-month month day-of-week`.

```
*/30 * * * *        → every 30 minutes
0 9 * * 1-5         → weekdays at 9am
0 0 * * 0           → every Sunday at midnight
0 */2 * * *         → every 2 hours
```

No seconds field. No `@reboot` or non-standard extensions.

## Multiple missions

A project has a **list of missions**, each independently scheduled. Like a crontab file with multiple entries:

```
# Mission 1: Architecture review — daily at 9am
0 9 * * *   architect   "Review code changes and update dependency graphs"

# Mission 2: Disk cleanup — every Sunday
0 0 * * 0   ling        "Analyze disk usage and suggest cleanup"

# Mission 3: Status check — every 30 minutes
*/30 * * * * ling       "Check CI/CD status and report issues"
```

Each mission is independent — its own schedule, agent, prompt, and optional model. Missions can be enabled/disabled individually.

## Scheduler behavior

Background task evaluates all enabled missions against their cron schedules:

1. **Tick**: scheduler wakes periodically (every ~10s) and checks all enabled missions.
2. **Match**: for each mission whose cron expression matches the current time window, fire the prompt.
3. **Busy skip**: if the target agent is already running, skip this trigger and log it.
4. **Run record**: each trigger creates a standard `AgentRunRecord` (see `agents.md`).

### Deduplication

The scheduler tracks the last fire time per mission. A cron match only fires if the current minute differs from the last fire minute — prevents double-firing within the same tick window.

## Run history

Each mission trigger creates:
- An `AgentRunRecord` in `runs/` (standard format).
- A `mission_run` entry in `missions/{id}/runs.jsonl` linking the run to the mission.

```json
{ "run_id": "run-ling-1700000000-123456", "triggered_at": 1700000000, "status": "completed", "skipped": false }
```

Skipped triggers (agent busy) are also logged with `"skipped": true`.

## Safety

| Guard | Value | Rationale |
|:------|:------|:----------|
| Minimum interval | 1 minute | Cron can't express sub-minute; prevents runaway |
| Max triggers per mission | 100 per day | Caps runaway missions |
| Max concurrent missions | No hard limit | Busy-skip naturally throttles |
| `max_iters` | Per agent config | Bounds each triggered run |
| No mission = no triggers | — | Missions must be explicitly created |
| Disabled missions | Skip silently | `enabled: false` stops all triggers |

## Lifecycle

```
create → enabled → (triggers run on schedule) → disabled → delete
```

- **Create**: user defines schedule + agent + prompt via Web UI or API.
- **Enable/Disable**: toggle without deleting. Disabled missions keep their config and history.
- **Delete**: removes the mission. Run history is preserved.
- **Edit**: update schedule, prompt, model, or agent. Takes effect on next tick.

## API operations

| Operation | Description |
|:----------|:------------|
| List missions | All missions for a project (with status, last run) |
| Create mission | New mission with schedule + agent + prompt |
| Update mission | Edit any field (schedule, prompt, agent, model, enabled) |
| Delete mission | Remove mission (history preserved) |
| Mission runs | Run history for a specific mission |

## Implementation

| Module | Responsibility |
|:-------|:---------------|
| `mission_scheduler.rs` | Cron evaluation, tick loop, trigger firing |
| `project_store/missions.rs` | Mission CRUD, run history persistence |
| `server/missions_api.rs` | HTTP endpoints for mission management |
