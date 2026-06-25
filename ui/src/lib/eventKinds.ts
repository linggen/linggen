/**
 * Canonical list of UiEvent `kind` values exchanged between the Rust server
 * and the Web UI over the WebRTC transport.
 *
 * Changing this list is a wire-protocol change — the Rust side must add or
 * rename the matching `UI_KIND_*` constant in `server/mod.rs` (and wherever
 * control-channel messages such as `page_state` / `user_info` are emitted).
 *
 * The TS dispatcher uses this list for exhaustiveness checking, so any kind
 * the server sends that is missing here will fail to type-check on the
 * handler map. This is the single source of truth for the union type.
 */
export const EVENT_KINDS = [
  // Chat / streaming
  'message',
  'token',
  'text_segment',
  'content_block',
  'turn_complete',

  // Activity / lifecycle
  'activity',
  'queue',
  'run',

  // Interactive widgets
  'ask_user',
  'widget_resolved',

  // Notifications / fallbacks
  'notification',
  'model_fallback',
  'tool_progress',
  'app_launched',
  'working_folder',

  // Pet — pushed "speak" cue (global); the surface plays the voice.
  'pet_speak',
  // Pet expression — emotion / one-shot gesture for the avatar (global).
  'pet_express',
  // Pet presenter — per-peer push: does THIS surface hold the FCFS singleton
  // lock (render + voice) or stay blank.
  'yinyue_present',

  // Control-channel pushes (not strictly UiEvent, but routed through the same
  // dispatcher because they carry a `kind` discriminator).
  'page_state',
  'user_info',
  'room_chat',
] as const;

export type EventKind = (typeof EVENT_KINDS)[number];

/** Exhaustiveness helper — forces the switch/map to cover every `EventKind`. */
export function assertNever(x: never): never {
  throw new Error(`Unreachable: unexpected event kind ${JSON.stringify(x)}`);
}

/** Narrow an unknown kind string to a known `EventKind`, or null. */
export function asEventKind(kind: unknown): EventKind | null {
  return typeof kind === 'string' && (EVENT_KINDS as readonly string[]).includes(kind)
    ? (kind as EventKind)
    : null;
}
