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
import { normalizeAgentStatus } from './messageUtils';
import { agentTracker } from './agentTracker';
import { postToParent } from './parentFrame';

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
  // The table's handlers take their own kind's event; `kind` came off this item.
  (eventHandlers[kind] as (e: UiEvent) => void)(item);
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

/** Tell the parent skill page whether we can still hear the server.
 *
 *  A skill page has no transport of its own — it sees the agent only through
 *  this iframe. Without this it cannot tell "the agent went quiet" from "the
 *  data channel is down", and on 2026-09-01 Pulse called a 3-minute channel
 *  reconnect a failed gather while the run was healthy underneath. */
export function relayConnectionToSkillIframe(
  status: 'connected' | 'reconnecting' | 'disconnected',
): void {
  if (window.parent === window) return;
  postToParent({
    type: 'linggen-skill-event',
    event: 'connection',
    payload: { status },
  });
}

/** A sign of life for the parent page: the agent is working even when no
 *  text streams (thinking between tools, a queued turn starting). */
type ActivityKind = 'turn_start' | 'thinking' | 'tool';

/** At most one `activity` relay per session this often — a heartbeat, not
 *  a stream. A turn start always goes (it opens a new round). */
const ACTIVITY_RELAY_MS = 5000;
const _lastActivityRelay = new Map<string, number>();

function activityKind(item: UiEvent): ActivityKind | null {
  if (item.phase === 'done') return null;
  const own = ownActivity(item);
  // A subagent working is its parent inside a tool call (Task), never a
  // new round of the page's chat.
  if (own && isSubagentEvent(item)) return 'tool';
  return own;
}

function ownActivity(item: UiEvent): ActivityKind | null {
  if (item.kind === 'token') return item.data?.thinking === true ? 'thinking' : null;
  if (item.kind === 'content_block') return item.phase === 'start' && item.data?.block_type === 'tool_use' ? 'tool' : null;
  if (item.kind !== 'activity') return null;
  const status = normalizeAgentStatus(String(item.data?.status || ''));
  if (status === 'model_loading') return 'turn_start';
  if (status === 'thinking' || status === 'working') return 'thinking';
  if (status === 'calling_tool') return 'tool';
  return null;
}

/** The routing keys any payload may carry; which ones depends on the kind. */
type RoutingKeys = { parent_agent_id?: string | null; parent_id?: string | null; run_id?: string | null };

function isSubagentEvent(item: UiEvent): boolean {
  const keys = (item.data ?? {}) as RoutingKeys;
  if (keys.parent_agent_id || keys.parent_id) return true;
  const runId = keys.run_id ? String(keys.run_id) : '';
  return !!(agentTracker.getParent(runId) || agentTracker.getParent(String(item.agent_id || '')));
}

function relayActivity(item: UiEvent): void {
  const kind = activityKind(item);
  const sessionId = item.session_id;
  if (!kind || !sessionId) return;
  const now = Date.now();
  const last = _lastActivityRelay.get(sessionId) ?? 0;
  if (kind !== 'turn_start' && now - last < ACTIVITY_RELAY_MS) return;
  _lastActivityRelay.set(sessionId, now);
  postToParent({
    type: 'linggen-skill-event',
    event: 'activity',
    payload: { sessionId, kind },
  });
}

/** Global events a skill page acts on: its cloud save was pulled
 *  (`save_changed` — read the files again), an app's quest facts changed
 *  (`quests_changed` — read the quests again), and device topics (e.g.
 *  yinyue/unanswered: an ask nobody will answer). Returns true when handled. */
function relayGlobal(item: UiEvent): boolean {
  if (item.kind === 'skill_save_changed') {
    const skill = useSessionStore.getState().activeSkillName;
    if (!skill || item.data?.skill === skill) {
      postToParent({ type: 'linggen-skill-event', event: 'save_changed', payload: item.data ?? {} });
    }
    return true;
  }
  if (item.kind === 'quests_changed') {
    postToParent({ type: 'linggen-skill-event', event: 'quests_changed', payload: item.data ?? {} });
    return true;
  }
  if (item.kind === 'device_topic') {
    postToParent({ type: 'linggen-skill-event', event: 'device_topic', payload: item.data ?? {} });
    return true;
  }
  return false;
}

function relayToSkillIframe(item: UiEvent): void {
  if (window.parent === window) return;
  if (relayGlobal(item)) return;
  relayActivity(item);

  if (item.kind === 'token' && item.text) {
    postToParent({
      type: 'linggen-skill-event',
      event: 'stream_token',
      payload: { text: item.text, done: item.phase === 'done' },
    });
    return;
  }

  if (item.kind === 'turn_complete') {
    const msgs = useChatStore.getState().messages;
    const lastMsg = [...msgs].reverse().find((m) => m.role === 'agent');
    postToParent({
      type: 'linggen-skill-event',
      event: 'stream_end',
      payload: { text: lastMsg?.text || '' },
    });
    return;
  }

  if (item.kind === 'content_block') {
    postToParent({
      type: 'linggen-skill-event',
      event: 'content_block',
      payload: {
        phase: item.phase,
        tool: item.data?.tool,
        args: item.data?.args,
        blockId: item.data?.block_id,
        output: item.phase === 'update' ? item.data?.output : undefined,
      },
    });
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
