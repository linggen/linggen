/**
 * Event dispatcher — routes `UiEvent`s received over the WebRTC transport
 * to per-kind handlers that update Zustand stores.
 *
 * Layering:
 *   rtcTransport  →  dispatchEvent  →  eventHandlers/<kind>.ts  →  stores
 *
 * This file handles cross-cutting concerns only:
 *   - session-scope filtering (drop events from non-active sessions)
 *   - skill-iframe bridge (relay key events to the parent page)
 * The handler map in `./eventHandlers` does the actual work.
 */
import type { UiEvent } from '../types';
import { useSessionStore } from '../stores/sessionStore';
import { useChatStore } from '../stores/chatStore';
import { asEventKind } from './eventKinds';
import { eventHandlers } from './eventHandlers';

export { suppressPermissionSync } from './eventHandlers/_shared';
export { handleAskUser } from './eventHandlers';

export function dispatchEvent(item: UiEvent, sessionIdOverride?: string): void {
  if (!passesSessionFilter(item, sessionIdOverride)) return;
  relayToSkillIframe(item);

  const kind = asEventKind(item.kind);
  if (!kind) {
    // Unknown kind — server is emitting a wire type this build doesn't know.
    // Log once (not per-event) so version skew is visible without spamming.
    warnUnknownKind(item.kind);
    return;
  }
  eventHandlers[kind](item);
}

// ---------------------------------------------------------------------------
// Session-scope filtering
// ---------------------------------------------------------------------------

/** Events with `session_id === 'global'` are broadcast. Other session-scoped
 *  events are dropped unless they match the caller's active session.
 *
 *  `notification` is always allowed through regardless of session (toasts are
 *  global). Everything else is gated on session ownership. */
function passesSessionFilter(item: UiEvent, sessionIdOverride?: string): boolean {
  if (item.kind === 'notification') return true;
  if (!item.session_id || item.session_id === 'global') return true;
  // Prefer the live store over the transport's ref override: the ref lags a
  // render cycle behind a session switch, and the old session's channel keeps
  // delivering until its close handshake lands — that window streamed tokens
  // into the wrong session's visible list. The store updates synchronously
  // with the switch. The override still covers scoped surfaces (embed/skill
  // iframes) where no session is selected in the store.
  const effectiveSessionId = useSessionStore.getState().activeSessionId || sessionIdOverride;
  if (effectiveSessionId) return item.session_id === effectiveSessionId;
  // Session-scoped event, no active session — belongs to a skill app / scoped session.
  return false;
}

// ---------------------------------------------------------------------------
// Skill-iframe bridge — forward key events to parent page when embedded
// ---------------------------------------------------------------------------

function relayToSkillIframe(item: UiEvent): void {
  if (window.parent === window) return;

  if (item.kind === 'token' && item.text) {
    window.parent.postMessage({
      type: 'linggen-skill-event',
      event: 'stream_token',
      payload: { text: item.text, done: item.phase === 'done' },
    }, '*');
    return;
  }

  if (item.kind === 'turn_complete') {
    const msgs = useChatStore.getState().messages;
    const lastMsg = [...msgs].reverse().find((m) => m.role === 'agent');
    window.parent.postMessage({
      type: 'linggen-skill-event',
      event: 'stream_end',
      payload: { text: lastMsg?.text || '' },
    }, '*');
    return;
  }

  if (item.kind === 'content_block') {
    window.parent.postMessage({
      type: 'linggen-skill-event',
      event: 'content_block',
      payload: {
        phase: item.phase,
        tool: item.data?.tool,
        args: item.data?.args,
        blockId: item.data?.block_id,
        output: item.data?.output,
      },
    }, '*');
  }
}

// ---------------------------------------------------------------------------
// Unknown-kind logging — logged once per distinct kind to surface version skew.
// ---------------------------------------------------------------------------

const _warnedUnknownKinds = new Set<string>();
function warnUnknownKind(kind: unknown): void {
  const key = String(kind);
  if (_warnedUnknownKinds.has(key)) return;
  _warnedUnknownKinds.add(key);
  console.warn(`[eventDispatcher] Unknown event kind "${key}" — check EVENT_KINDS in eventKinds.ts`);
}
