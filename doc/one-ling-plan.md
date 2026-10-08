---
type: plan
reader: Coding agent
status: approved 2026-10-08 (Hanli: "都OK, go"); step 1 in progress
guide: |
  Build plan for one-ling-spec.md. Each slice ships and is checked on its own.
---

# One Ling — build plan

Spec: `one-ling-spec.md`. Repos: engine `linggen/src`, Mac UI `linggen/ui/src`,
phone `linggen-mobile/lib`, skills `skills/`.

## Already there — leave alone

- Mac dispatch: `server/chat/members.rs::default_responder` picks Ling;
  `handler.rs::route_target` handles `@name`.
- One-hop forward guard: `engine/tools/builtin/agent_chat.rs`.
- Relay label from `from_id`: `labeled_msg`, `thread.rs::row_for`,
  `ChatMessageList.tsx::agentLabel`.
- Phone → Mac hand-off: `ask_mac_app` → `LingChatController.askSkill` →
  `_skillSession` (latest app session).
- DJ download is a typed call (`QueueTracks`) behind the user's confirm card —
  **the card stays** (Hanli 2026-10-08).
- Retained topics `server/api/topic.rs`; publish loop precedent
  `perception/publish.rs`; phone `tool_catalog_publisher.dart`; flag precedent
  `SkillToolDef.pet` / `pet_tools`.
- Persona: `YinyueSoul.assemble`, baked by `tool/sync_persona.sh`.

## Slices

1a. **Phone tool names `<app>_<verb>`.** DJ's unprefixed tools,
    `photo_sync`/`photo_backup` → `photos_*`, `device_scan` → `shifu_scan`.
    Agent/shell tools keep their names. Assert in `AppTool`. Map old names on
    `thread.json` load; ActionQueue accepts the old `<app>-<tool>` op for one
    release; update `dj_carplay_bridge.dart`, `skills/apple-shifu`
    phone-actions callers, persona text; chips show `dj.play`.
1b. **Pack signal = last app screen** before opening Chat (≈5 min), a table
    not a switch; keep DJ-playing, recent-use, Health-word signals.
1c. **Engine `mac/tools`.** `remote: bool` on `SkillToolDef`, `remote_tools()`,
    `server/mac_tools.rs` publishes retained `mac/tools` with `<app>_<verb>`
    names; generic, no skill names in Rust. DJ opts in its read and playlist
    tools.
2.  **Phone reads `mac/tools`** → `requiresMac` tools calling
    `/api/skills/{app}/tools/{tool}` over RTC; tiers read/edit/admin →
    read/edit/destructive; chips `Mac · …`; grey when the Mac drops.
3.  **Ling persona single-sourced**: `sync_persona.sh` bakes `agents/ling.md` +
    `assets/places/phone-ling.md`; hand-off and "Mac unreachable: say so and
    stop" move from Yinyue to Ling; `yinyue.md` loses task parts.
4.  **Thread speakers**: wire messages carry `speaker` (missing = yinyue);
    `screens/chat/table.dart` ports `row_for`.
5.  **Dispatch**: `screens/chat/dispatch.dart` (Ling unless `@Yinyue`/`@银月`;
    pre-fill, 10-min clear); turn per member; `forward` tool, one hop,
    `ChatRole.forward` line; heralds stay hers.
6.  **Engine `via`** on `ChatRequest`/`ChatMsg`: "Ling continuing her task from
    the phone" — new `labeled_msg` and `row_for` branches; UI "Ling · from
    iPhone". (`handler.rs` drops `sender == target`, so this needs its own
    field.)
7.  **Hand-off is Ling's step**: `ask_mac_app` → `continue_on_mac`, live card
    `Mac · DJ` replaces `ChatRole.app`/`relay_card.dart`; unreachable → one line,
    stop.
8.  **One Chat screen**: remove `HomeShell.lingTab`; Chat is home; Mac sessions
    stay as a read-only picker.
9.  **Mac composer pre-fill** `@Yinyue` in `ChatInput.tsx`, same clear.

Later: **Lingjing on the phone** — needs 4, 5, 8 + cloud save; pure-Dart rules
held to shared fixtures; stage-first UI per
`skills/lingjing/doc/phone-prototype.html`; a skill-moment hook gives Yinyue a
turn.

10. **Yinyue appears nowhere until found in Lingjing** (Hanli 2026-10-08: needed
    — every app shares one Chat on the phone, and her being there from the
    start breaks Lingjing's story). Not on the Mac (pet window, 3D body, menubar
    face, chat, Settings sections, heralds) nor the phone (Chat, Settings).
    Today the gate (`absent_until` → `server/chat/presence.rs`) covers only
    Lingjing's own sessions. Shape: a global "met Yinyue" flag set by the
    Lingjing save, declared in Yinyue's own agent config (no skill names in
    Rust), synced to the phone (cloud save); before it, Chat is Ling only.
    Needs 5 and 8. Settled: **no Lingjing, no Yinyue** — a user who never plays
    never sees her. The fresh-install guide (2026-09-25) has Ling, not her,
    offer 开府. Site and App Store copy follow later.

## Risks

- Rename blast radius (queue ops, stored threads, CarPlay, Mac callers).
- Old threads without `speaker` load as Yinyue's.
- A stale `mac/tools` must go grey when the Mac drops.
