/**
 * Transport abstraction — decouples the UI from the underlying connection mechanism.
 *
 * Events flow bidirectionally between the linggen server and the browser over
 * WebRTC data channels. The UI sends requests and receives events through this interface.
 */
import type { UiEvent } from '../types';

// ---------------------------------------------------------------------------
// Types
// ---------------------------------------------------------------------------

/** Status of the transport connection. */
export type TransportStatus = 'disconnected' | 'connecting' | 'connected' | 'reconnecting';

/** Callbacks the transport invokes. */
export interface TransportCallbacks {
  /** Called when a server event arrives for a session. */
  onEvent: (sessionId: string | null, event: UiEvent) => void;
  /** Called when the transport connection status changes. */
  onStatusChange: (status: TransportStatus) => void;
  /** Called on reconnect (not initial connect) — consumer should resync state. */
  onReconnect?: () => void;
  /** Called when an event fails to parse. */
  onParseError?: () => void;
}

/** A request to send a chat message (mirrors POST /api/chat body). */
export interface ChatRequest {
  project_root: string;
  agent_id: string;
  message: string;
  session_id?: string | null;
  mission_id?: string;
  model_id?: string;
  images?: string[];
}

/** A request to respond to an AskUser prompt. */
export interface AskUserResponse {
  question_id: string;
  answers: any[];
  session_id?: string | null;
}

/** A plan action request. */
export interface PlanAction {
  type: 'approve' | 'reject' | 'edit';
  project_root: string;
  session_id?: string | null;
  agent_id?: string;
  edited_plan?: string;
}

// ---------------------------------------------------------------------------
// Transport interface
// ---------------------------------------------------------------------------

/**
 * A Transport carries events between the linggen server and browser.
 *
 * Lifecycle:
 *   1. Create a transport instance with callbacks.
 *   2. Call connect() — the transport establishes the underlying connection.
 *   3. Call subscribeSession(id) to start receiving events for a session.
 *   4. Call send*() methods to send requests.
 *   5. Call unsubscribeSession(id) when done with a session.
 *   6. Call disconnect() to tear down the connection.
 */
export interface Transport {
  /** Establish the transport connection. */
  connect(): void;

  /** Tear down the transport connection. */
  disconnect(): void;

  /** Subscribe to events for a session. */
  subscribeSession(sessionId: string): void;

  /** Unsubscribe from a session's events. */
  unsubscribeSession(sessionId: string): void;

  /** Current connection status. */
  status(): TransportStatus;

  // --- Outbound requests ---

  /** Send a chat message. */
  sendChat(req: ChatRequest): Promise<{ session_id?: string; status?: string }>;

  /** Respond to an AskUser prompt. */
  sendAskUserResponse(req: AskUserResponse): Promise<void>;

  /** Send a plan action (approve / reject / edit). */
  sendPlanAction(req: PlanAction): Promise<void>;

  /** Clear chat history for a session. */
  sendClear(projectRoot: string, sessionId?: string | null): Promise<void>;

  /** Compact chat context for a session. */
  sendCompact(projectRoot: string, sessionId: string | null, agentId: string, focus?: string): Promise<{ compacted?: boolean; referenced_files?: string[] }>;

  /** Proxy an HTTP request through the transport (for remote mode).
   *  Returns { status, body } where body is the raw response text. */
  httpProxy(method: string, url: string, body?: any): Promise<{ status: number; body: string }>;

  /** Tell the server which session/project the frontend has active.
   *  The server uses this to scope its page_state push. */
  sendViewContext(ctx: { sessionId: string | null; projectRoot: string | null; view: 'main' | 'embed' | 'consumer' }): void;

  /** Send a room chat message (fire-and-forget). */
  sendRoomChat?(text: string, senderName: string): void;

  /** Join the Yinyue presenter registry — this surface can render her. The
   *  server grants the singleton lock to the first subscriber (FCFS). */
  sendYinyueSubscribe?(): void;
  /** Leave the Yinyue presenter registry (on unmount / before unload). */
  sendYinyueRelease?(): void;
}

// ---------------------------------------------------------------------------
// Singleton transport instance
// ---------------------------------------------------------------------------

let _transport: Transport | null = null;

/** Get the global transport instance. */
export function getTransport(): Transport {
  if (!_transport) {
    throw new Error('Transport not initialized — call setTransport() first');
  }
  return _transport;
}

/** Set the global transport instance. */
export function setTransport(transport: Transport): void {
  if (_transport) {
    _transport.disconnect();
  }
  _transport = transport;
}
