/**
 * Transport abstraction — decouples the UI from the underlying connection mechanism.
 *
 * Transports carry chat events bidirectionally between the linggen server and the browser.
 * The UI is transport-agnostic: it sends requests and receives events through this interface
 * regardless of whether the underlying pipe is SSE+fetch, WebRTC data channels, or anything else.
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
  clear_context?: boolean;
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
  /** Establish the transport connection (SSE EventSource, WebRTC peer connection, etc.). */
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
}

// ---------------------------------------------------------------------------
// SSE Transport implementation
// ---------------------------------------------------------------------------

/**
 * SSE transport — wraps the existing EventSource + fetch pattern.
 *
 * Inbound:  EventSource on /api/events?session_id=... receives server events.
 * Outbound: fetch POST to /api/chat, /api/ask-user-response, etc.
 */
export class SseTransport implements Transport {
  private callbacks: TransportCallbacks;
  private eventSource: EventSource | null = null;
  private currentSessionId: string | null = null;
  private lastSeq = 0;
  private hadConnection = false;
  private _status: TransportStatus = 'disconnected';

  constructor(callbacks: TransportCallbacks) {
    this.callbacks = callbacks;
  }

  connect(): void {
    // SSE connects per-session via subscribeSession; initial connect is a no-op.
    // If no session is subscribed yet, open a global connection.
    if (!this.currentSessionId) {
      this.openEventSource(null);
    }
  }

  disconnect(): void {
    this.closeEventSource();
    this.setStatus('disconnected');
  }

  subscribeSession(sessionId: string): void {
    if (this.currentSessionId === sessionId) return;
    this.closeEventSource();
    this.currentSessionId = sessionId;
    this.openEventSource(sessionId);
  }

  unsubscribeSession(_sessionId: string): void {
    // SSE only supports one session at a time — unsubscribe is a no-op.
    // The next subscribeSession() will reconnect.
  }

  status(): TransportStatus {
    return this._status;
  }

  // --- Outbound ---

  async sendChat(req: ChatRequest): Promise<{ session_id?: string; status?: string }> {
    const resp = await fetch('/api/chat', {
      method: 'POST',
      headers: { 'Content-Type': 'application/json' },
      body: JSON.stringify(req),
    });
    return resp.json();
  }

  async sendAskUserResponse(req: AskUserResponse): Promise<void> {
    await fetch('/api/ask-user-response', {
      method: 'POST',
      headers: { 'Content-Type': 'application/json' },
      body: JSON.stringify(req),
    });
  }

  async sendPlanAction(req: PlanAction): Promise<void> {
    const { type, edited_plan, ...rest } = req;
    const url = `/api/plan/${type}`;
    await fetch(url, {
      method: 'POST',
      headers: { 'Content-Type': 'application/json' },
      body: JSON.stringify({ ...rest, ...(edited_plan ? { edited_plan } : {}) }),
    });
  }

  async sendClear(projectRoot: string, sessionId?: string | null): Promise<void> {
    await fetch('/api/chat/clear', {
      method: 'POST',
      headers: { 'Content-Type': 'application/json' },
      body: JSON.stringify({ project_root: projectRoot, session_id: sessionId }),
    });
  }

  async sendCompact(projectRoot: string, sessionId: string | null, agentId: string, focus?: string): Promise<{ compacted?: boolean; referenced_files?: string[] }> {
    const resp = await fetch('/api/chat/compact', {
      method: 'POST',
      headers: { 'Content-Type': 'application/json' },
      body: JSON.stringify({ project_root: projectRoot, session_id: sessionId, agent_id: agentId, focus }),
    });
    return resp.json();
  }

  async httpProxy(_method: string, url: string, body?: any): Promise<{ status: number; body: string }> {
    // SSE transport has direct HTTP access — just fetch normally
    const opts: RequestInit = { method: _method };
    if (body && _method !== 'GET') {
      opts.headers = { 'Content-Type': 'application/json' };
      opts.body = typeof body === 'string' ? body : JSON.stringify(body);
    }
    const resp = await fetch(url, opts);
    const text = await resp.text();
    return { status: resp.status, body: text };
  }

  // --- Internal ---

  private openEventSource(sessionId: string | null): void {
    this.setStatus('connecting');
    const url = sessionId
      ? `/api/events?session_id=${encodeURIComponent(sessionId)}`
      : '/api/events';
    const es = new EventSource(url);

    es.onopen = () => {
      const wasReconnecting = this.hadConnection;
      this.lastSeq = 0;
      this.hadConnection = true;
      this.setStatus('connected');

      if (wasReconnecting) {
        this.callbacks.onReconnect?.();
      }
    };

    es.onerror = () => {
      if (this.hadConnection) {
        this.setStatus('reconnecting');
      }
    };

    es.onmessage = (e) => {
      try {
        const item = JSON.parse(e.data) as UiEvent;
        if (typeof item.seq === 'number') {
          if (item.seq <= this.lastSeq) return;
          this.lastSeq = item.seq;
        }
        this.callbacks.onEvent(this.currentSessionId, item);
      } catch (err) {
        console.error('SSE parse error', err);
        this.callbacks.onParseError?.();
      }
    };

    this.eventSource = es;
  }

  private closeEventSource(): void {
    if (this.eventSource) {
      this.eventSource.close();
      this.eventSource = null;
      // Don't reset hadConnection here — preserve it across session switches
      // so that onReconnect fires correctly after a session switch + reconnect.
    }
  }

  private setStatus(status: TransportStatus): void {
    this._status = status;
    this.callbacks.onStatusChange(status);
  }
}

// ---------------------------------------------------------------------------
// Singleton transport instance
// ---------------------------------------------------------------------------

let _transport: Transport | null = null;

/** Get or create the global transport instance. */
export function getTransport(callbacks?: TransportCallbacks): Transport {
  if (!_transport && callbacks) {
    _transport = new SseTransport(callbacks);
  }
  if (!_transport) {
    throw new Error('Transport not initialized — call getTransport(callbacks) first');
  }
  return _transport;
}

/** Replace the global transport (e.g., switch from SSE to WebRTC). */
export function setTransport(transport: Transport): void {
  if (_transport) {
    _transport.disconnect();
  }
  _transport = transport;
}
