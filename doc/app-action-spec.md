---
type: spec
reader: Coding agent and users
guide: |
  Product specification — describe what the system should do and why.
  Keep it brief. Aim to guide design and implementation, not document code.
  Avoid implementation details like function signatures, variable types, or code snippets.
---

# App Actions

Every user-visible app action is a declared tool. Both agents can reach every action: the side that owns an action runs it locally; the other side triggers it through a typed cross-device call. Settled 2026-07-31.

## Related docs

- `tool-spec.md`: built-in tools, dispatch, permission gate.
- `skill-spec.md`: skill format, custom tool declarations.
- `yinyue-companion-spec.md`: Yinyue's agent surface.
- `../../linggen-mobile/doc/tech-spec.md`: phone app structure, and one doc per lane beside it.

## The symmetry rule

Both Ling and Yinyue know how to do everything. For each action:

- The **owning side** (where the data or OS capability lives) declares a local tool and executes it.
- The **other side** reaches it as a typed call routed to the owner — never by re-phrasing through the other agent.
- `agent_chat` relay is reserved for **judgment-shaped asks** ("get me the karaoke version") where the receiving agent's intelligence is the point. Exact-param mutations never pass through a second model.
- User buttons are never gated. Tiers gate agents only.

## One writer per mutation (Mac)

Every mutation is **one script with two callers**:

- Agent: SKILL.md tool `cmd:` → script.
- Web page: button → `/api/bash` → the same script.

Page JS is rendering only — it computes nothing it then writes. `PageUpdate` remains for proposals and display (tracklists, insights, suggestions) and stops being a write path. This is the schema law in executable form: one writer, callers can't drift.

## Script language and runtime

Scripts that carry JSON-heavy app logic are **JavaScript, run by a bundled bun** (`~/.linggen/bin/bun`, installed by install.sh). Rationale, once: existing page write logic lifts over verbatim instead of being re-fought in bash/jq/python (macOS ships neither jq nor a prompt-free python3), and bun is one self-contained binary — disk-heavy (~90MB, one-time), runtime-light (no daemon, per-call process, ~15ms start). Simple non-JSON verbs may stay plain sh.

## Tool declarations (Mac)

The existing SKILL.md `tools:` schema is the declaration format — name, description, cmd, tier, timeout_ms (Pulse is the reference density).

- **One tool per verb.** Merge two verbs into one tool with an op field only when their param sets are identical. Never a noun tool with a mixed-param op enum: descriptions stay focused, schemas stay fully-required (strict-mode capable), and no optional arg can decay into a `{{placeholder}}` literal.
- **Descriptions teach usage**: when to reach for it, args, return shape, how to judge results.
- Tool lists stay small because tools load per app session (DJ ~9, CFO ~7, media ~10).

## Phone tool system

The phone has no SKILL.md; its equivalent is a **compiled Dart ToolRegistry**. A declaration is one record with the same schema fields — app, name, description, params, tier, `requiresMac`, plus a handler closure.

- One `tools.dart` per app module (dj, photos, cfo, shifu), registered at startup **against the service layer** (library, sync controller, repo — never screens), so tools work when the tab was never opened. Handlers are the same methods the buttons call.
- **Yinyue's chat loop reads the registry.** Her existing tools (memory_note, open_tab, get_environment, web_search, photo_sync, mac_apps, ask_mac_app, memory_recall) migrate in as the shell's entries; `requiresMac` replaces the ad-hoc availability gate.
- The **same registry serves the Mac's queue**: an inbound envelope dispatches through identical code. The catalog Ling sees is the registry serialized minus handlers.

## Tiers and confirms

Three tiers: `read`, `edit`, `destructive`.

- `read` runs silently; `edit` rides the normal permission model.
- `destructive` requires a **human confirm on the device that executes**, regardless of which agent called: in Yinyue's chat an AskUser card; for a Mac-queued action, a confirm on the phone. An OS-owned confirm (PhotoKit delete sheet) counts as the confirm — never double-prompt.
- A tool's tier must match its effect. A remote-mutating tool is not `read` (fix: SyncPhone).

## Cross-device calls

- **Shared data never crosses as an action.** Each side mutates its own copy through its local tool; LWW/CAS sync propagates (Ling saves a playlist via the Mac script; the phone pulls). This removes most cross-device traffic by construction.
- **Typed queue for device-exclusive verbs only** (~3 today: PhotoKit delete, backup upload, device scan). Envelope `{app, tool, params}` over the retained-topic transport names the same tool the local agent would call; it drains on app resume. `sync-requested` is the precedent and folds into this shape.
- **Catalog**: the phone publishes its registry as a retained `phone/tools` topic on connect (same mechanism as `shifu/readout`). The Mac never requests it — reads are published, actions are queued.

## Visibility

Every tool call is visible where it runs:

- In Yinyue's chat, each call renders an **inline chip** — tool name, one-line args, live status, tap to expand args/result. Long-running tools use the live task card. The chip shows the process; she speaks only the outcome — her no-process-narration voice rules are unchanged.
- Destructive calls render the AskUser card first, then the chip.
- Mac-queued runs render the same chip prefixed as the Mac's ask — nothing runs invisibly on either side.

## Rollout

Close the audit gaps by declaring tools, app by app:

- **DJ**: playlist create/rename/delete/add/remove/reorder, delete tracks (Mac scripts + phone handlers over the shared register).
- **CFO**: budgets set/remove, recategorize, import/undo (phone handlers; Mac side stays read + propose until the store owns a Mac writer).
- **Photos/media**: backup, delete-queue, find-broken as declared tools on their owning sides; `SyncPhone` re-tiered.
- **Shifu**: phone scan as a phone tool; Mac cleanup stays recommend-only.
