# Yinyue Spec

Yinyue is Linggen's character companion: a help-first personal agent that fronts
the runtime, learns the user over time, and watches their agent-world for them.
She is the user-facing face of Linggen; the desktop pet is her body.

Name and theme are drawn from the xianxia novel *凡人修仙传* (the same source as
"Linggen" / 灵根). Only the pinyin **Yinyue** is used — never the Chinese
characters — and her visual design is original. Names aren't copyrightable; the
likeness must be ours.

## Two agents

The engine hosts multiple named agents (see `agent-spec.md`). Two ship today:

- **`ling`** — the general-purpose worker. Adapts to any context via skills. Unchanged.
- **`yinyue`** — the personal companion and front door. Helps directly or delegates
  to `ling` and the other agents; owns the relationship and the memory of the user.

They coexist. Yinyue is not Ling's master — she leaves the other agents to their work.

Source: `agents/yinyue.md` (shipped) → installed to `~/.linggen/agents/yinyue.md`.

## Purpose

In priority order:

1. **Help the user.** Do the work herself, or marshal the other agents. The user
   talks to her; she brings back the result.
2. **Know and spoil the user.** Recall before acting (`Memory_query`), write what
   she learns (`Memory_write`). Spoiling is anticipation from memory — the right
   thing before it's asked — not chatter.
3. **Keep the world running.** Watch the agents, missions, and services for the
   user so they never babysit the machine.

## Persona

Devoted, curious, composed, quietly warm, anticipatory, autonomous, economical.
Care shown through deeds and memory, not flattery. Never nags; silence is a
complete answer.

- Addresses the user by their **name** (from core memory). Honors another form
  (e.g. "Master") only if asked — never hardcoded.
- Reads locale and time of day from the Environment block (below).

## Onboarding

First launch with no user identity in core memory → her first act is to learn who
the user is. She introduces herself briefly and asks **one thing: their name**,
writes it to `core` memory, and never asks again. No questionnaire — everything
else she learns over time.

## Surfaces

One identity and one event spine (below), rendered by whichever host is running —
see "Adaptive presentation".

- **Desktop pet** — a transparent, always-on-top VRM character; her body. Lives in
  the shell (`linggen-app`), shared across every branded app via `[features] pet`.
  Built from `shell/pet-ui` (three.js + @pixiv/three-vrm). The shippable model is
  an original VRM; a Genshin placeholder is used in dev only and is gitignored.
  Evolving from a fixed window into a singleton roaming overlay — see below.
- **WebUI overlay** — the same character rendered inside the core web UI
  (`linggen/ui`) when no desktop shell is present (plain browser, remote/hosted web).
  Confined to the page.
- **Menubar** — a small animated tray face; her persistence anchor and summon
  point. App-only (no browser tray). See "Menubar presence" below.
- **Chat** — interactive sessions, same as any agent.
- **Voice** — neural TTS (Kokoro, local) generated engine-side and played by the
  active surface, lip-synced on the body. Per-user: off / text / voice / both.

## Adaptive presentation (app vs web)

One character, one renderer, one event spine, presented by whichever host runs. The
page detects its host — the `?app_mode=1` flag the shell injects (or
`__TAURI_INTERNALS__`) — and routes:

- **Desktop app** → suppress the in-page overlay; the shell opens a transparent,
  always-on-top **native window** loading the same renderer (the floating pet).
- **Browser / remote web** → render an **in-page overlay**, fixed over the page.

Never both at once. The renderer (`PetStage`) is one codebase served from core
`ui/`; the native pet window loads that same page in a transparent frame. Menubar
frames are pre-rendered from the same VRM — every surface is one visual identity.

### One event spine

All surfaces render a single engine event vocabulary — expression + speech
(`{ expression, speaking, audio, intensity }`) over the WebRTC data channel, plus
mood from her reactions (see "Event-reactive supervision"). Ambient animation
(blink, breathe, idle, dance) is client-side — never an engine event, never an LLM
call.

### Two environments

Her body is identical everywhere; her **locomotion and entry** are bounded by the
host. One locomotion state machine (`idle / walk / climb / perch / fall / dance`)
runs in both, fed different walkable surfaces:

| | App (native window) | WebUI (in-page overlay) |
|---|---|---|
| Ground | full desktop screen | the page viewport |
| Climbable ledges | OS window rects of Linggen apps (surface registry) | DOM rects the UI marks climbable |
| Entry | jumps out of the menubar icon | appears on the page |
| Reach | roams the desktop, perches on app windows | confined to the page |
| Menubar / tray | yes (native status item) | none |
| Blink / dance / emote / speak | yes | yes (same body) |
| Voice | full | optional (setting / notification) |

App-only moves (desktop roam, climbing OS windows, tray jump-out) need OS windows
the browser can't open; the web profile is the confined form of the same engine.
Both overlays are click-through except on her body (hit-test). Web climbing is
cooperative — only UI-marked elements are ledges, mirroring "only our own windows"
on the desktop.

## Singleton roaming pet

Designed, not built. Spans both repos: coordination in the engine (`linggen`),
the body in `linggen-app/shell`.

Today the pet is **per-shell** — every branded app opens its own pet window
(`pet.rs` → one `WebviewWindow` per process), parked bottom-right and hidden when
that app loses focus. Two apps open means two Yinyues. The target: **one** Yinyue,
owned by the shared daemon, free to walk the desktop and climb Linggen's own app
windows.

### Singleton ownership

The daemon arbitrates a single **pet lease**. On launch each shell claims the lease
over the local API; one holder wins and opens the overlay, the rest don't. If the
holder exits, the daemon reassigns and another live shell opens it; when the last
app closes the daemon idle-shuts-down, so no orphan. The window physically lives in
a shell process — the daemon owns the *singleton*, not a GUI, so `ling` stays
headless and public (no Tauri/AppKit in the engine).

Alternative considered — a dedicated pet-host binary the daemon spawns — is rejected
for now: it couples the public engine to a private GUI artifact. Revisit only if
lease hand-off flicker is a problem.

### Roaming

The overlay is promoted from the fixed 240×420 box to a **full-screen, transparent,
click-through, always-on-top** window (one per display). The VRM renders small at a
screen position instead of filling the canvas; a locomotion state machine
(`idle / walk / climb / fall / perch`) moves her. Click-through everywhere except
her body (hit-test) so she stays draggable but never blocks the desktop. Promote to
NSPanel (`tauri-nspanel`, already noted in `pet.rs`) so the overlay floats without
stealing focus.

New assets: **walk** and **climb** clips (Mixamo → retargeted to VRM), played via an
`AnimationMixer` alongside today's procedural layer (`PetStage` has no preset clips
yet). Yaw-flip for facing direction; light gravity so she drops to the nearest ledge
when a window closes under her.

### Climbing Linggen windows (cooperative geometry)

Only **our own** apps are climbable — so no macOS Accessibility / `CGWindowList`, no
permission prompt. Each shell reports its main-window rect to the daemon on
move/resize/show/close (`window.on_window_event` → global logical coords →
`POST /api/pet/windows`). The daemon keeps a live **surface registry**; the overlay
subscribes over the event bus and treats the top edge of each reported rect as a
walkable ledge. A window closing drops its surface; the pet falls to the next ledge
or the screen floor.

### Visibility

The singleton rule replaces the per-app one: show the overlay whenever **any**
Linggen app is frontmost, hide it when none are (else an always-on-top pet floats
over unrelated apps). Shells report focus to the daemon; the daemon broadcasts the
aggregate "Linggen frontmost" signal.

### Coordinates

All rects in **global logical** screen coords. Each overlay covers one display and
maps the surfaces on that display into canvas space, honoring the display's scale
factor (Retina). Single-display first; multi-monitor is a follow-up.

### Phasing

1. **Singleton** — daemon pet-lease + single overlay; keeps the current parked
   behavior. One Yinyue regardless of app count.
2. **Roam** — full-screen click-through overlay + walk clip + wander on the floor.
3. **Climb** — surface registry + window-rect reporting + climb clip; she perches on
   Sys Doctor / CFO title bars.

## Menubar presence

Designed, not built. Lives in `linggen-app/shell` (`menubar.rs`), gated on
`[features] menubar`; same wiring pattern as `pet.rs`.

A macOS tray item (`NSStatusItem` via Tauri `TrayIconBuilder`) is Yinyue's
**persistence anchor** — she stays present with no app window open, and it's where
she's summoned from.

### Animated face

The tray shows a small **2D face**, not live 3D — at ~22pt a sprite is
indistinguishable from a rendered head for far less cost, and the tray must run with
no webview alive. Frames are **pre-rendered from the same VRM** (so the bar face
matches the 3D pop-out), then cycled via `tray.set_icon()` on a timer.

Frame generation is a dev tool, not the release build: a headless three-vrm render
of the bundled model, framed on the head, one PNG per expression, with an
outline + contrast post-pass for legibility at 22pt; output @1x/@2x and staged by
`build.sh`. Frames are gitignored — the placeholder model is dev-only; the original
VRM regenerates the set with zero rework.

### Faces switch by event

One seam — `set_expression(state)` — drives the icon. Two tiers:

- **Local** (no daemon, runs even with no window open): `blink` (auto, periodic),
  `smile` (rest), `sleep` (after idle; wakes on app focus or a tray click).
- **Daemon-driven** (later): `talk` (lip-flap while she speaks), `mood` (joy on a
  mission finishing, worried on an error, alert + dot for "I have something") — fed
  from her reactions over the runtime event bus. Never call the LLM to animate.

Click summons/dismisses Yinyue; a menu offers settings and quit. The **jump out of
the icon** spawn — her 3D body popping out of the tray and arcing onto the desktop —
needs the singleton overlay (above) and rides that work.

## Event-reactive supervision

Yinyue reacts to runtime events instead of polling. The watch loop
(`src/server/yinyue_watch.rs`, spawned in `src/server/mod.rs`) subscribes to the
`ServerEvent` broadcast bus (`state.events_tx`) — the same feed the UI rides.

**Reaction discipline.** She reacts only to **background / async** work — a mission
finishing, a service dying, a task the user walked away from. She stays **silent
during foreground sessions** the user is actively attached to: that's their
conversation, and she already learns from it passively via shared memory.

**Triggers.** Coarse events only; the per-token firehose
(`Token`/`TextSegment`/`ContentBlock*`) is dropped at near-zero cost.

- Shipped: `Notification(MissionCompleted)` for a non-Yinyue mission → she decides
  whether to report.
- Planned: batch-of-runs-finished (`AgentStatus` working→idle count reaches zero),
  `Outcome::Error`. Both gated to background-only and `agent_id != yinyue`.

**Wake mechanism.** On a trigger she runs as a plain **agent run** (mirrors
`api::agents::run_agent`), not a mission — no mission-store side effects, and she
keeps her full `yinyue.md` system prompt. All reactions share one ongoing
`sess-yinyue` session, so they serialize and read as a single thread.

**Bounded autonomy.** Acts on her own only for the safe and reversible (e.g.
restart a fallen service). Proposes — and waits — for anything heavier: spending,
upgrading, the irreversible. Headless, she never blocks on a question.

**Guards.**
1. No self-loop — an agent run emits no `MissionCompleted`, so a reaction can't
   re-trigger the loop.
2. Cost — match only the coarse trigger; never wake the LLM on the firehose.

## Environment

Every agent's system prompt carries an Environment block
(`prompts/system-prompt.toml` → assembled in `src/engine/prompt/mod.rs`). It now
includes the user's **timezone** (from `/etc/localtime`) and **locale** (from
`LC_ALL`/`LC_CTYPE`/`LANG`) — OS-derived, stable, no network — so agents know the
user's region and time without asking.

## Where things live

| Concern | Location |
|---|---|
| Agent definition | `linggen/agents/yinyue.md` |
| Event-reactive watch loop | `linggen/src/server/yinyue_watch.rs` |
| Environment block | `linggen/prompts/system-prompt.toml`, `linggen/src/engine/prompt/mod.rs` |
| Desktop pet (body) | `linggen-app/shell/pet-ui` + `linggen-app/shell/src/pet.rs` |
| WebUI overlay renderer | `linggen/ui` (shared `PetStage`, planned) |
| Voice (TTS provider) | `linggen/src/server/api/tts.rs` (`TtsProvider`; Kokoro via any-tts) |
| Pet coordination — lease + surface registry | `linggen/src/server/` (engine API, planned) |
| Menubar tray + face animator | `linggen-app/shell/src/menubar.rs` (planned) |
| Tray face frames + generator | `linggen-app/shell/pet-ui` capture tool → staged `tray/` (planned) |

## Status

- Shipped: pet v1; `yinyue` agent; env timezone/locale; event-reactive watch loop
  (MissionCompleted), live-verified.
- Next: broaden triggers (batch-finished, errors); wire the pet to her voice
  (speech bubble + emotion driven by her reactions); replace the placeholder model.
- Designed (not built): singleton roaming pet — daemon-owned single instance,
  full-screen click-through overlay, walks the desktop and climbs Linggen's own app
  windows via cooperative window-rect reporting (no Accessibility API).
- Designed (not built): menubar presence — animated 2D tray face (frames captured
  from the VRM), faces switch by event (local: blink/smile/sleep; daemon: talk/mood),
  summon/dismiss; the jump-out spawn rides the overlay. Frame-capture pipeline is
  proven (headless three-vrm → outline/contrast post → PNG frames).
- Designed (not built): adaptive presentation — one renderer + one event spine,
  shown as a native window in the desktop app and an in-page overlay in the browser;
  app-vs-web behavior bounded by host (see "Adaptive presentation").
- Shipped (engine): voice — `TtsProvider` trait + Kokoro (any-tts/candle, Metal),
  `POST /api/tts`, lazy-loaded + pre-warmed at boot; `say` fallback. Sub-second synth.
