/**
 * WebRTC transport — uses data channels for bidirectional chat events.
 *
 * Signaling is pluggable via SignalingStrategy:
 *   Local:  WhipSignaling — single POST to /api/rtc/whip
 *   Remote: RelaySignaling — POST offer to relay, poll for answer
 *
 * Each session gets its own data channel for natural isolation.
 */
import type { UiEvent } from '../types';
import type {
  Transport,
  TransportCallbacks,
  TransportStatus,
  ChatRequest,
  AskUserResponse,
  PlanAction,
} from './transport';
import { WhipSignaling, type SignalingStrategy } from './signaling';

/** Configuration for the WebRTC transport. */
export interface RtcTransportConfig {
  /** Signaling strategy. Default: WhipSignaling('/api/rtc/whip'). */
  signaling?: SignalingStrategy;
  /** ICE servers for NAT traversal. */
  iceServers?: RTCIceServer[];
}

const DEFAULT_ICE_SERVERS: RTCIceServer[] = [
  { urls: 'stun:stun.l.google.com:19302' },
  { urls: 'stun:stun.cloudflare.com:3478' },
];

/**
 * WebRTC transport implementation.
 *
 * Data channels:
 *   - "control": session lifecycle, heartbeat, http-proxy requests
 *   - "sess-{id}": per-session chat events (bidirectional)
 */
export class RtcTransport implements Transport {
  private callbacks: TransportCallbacks;
  private config: RtcTransportConfig;
  private pc: RTCPeerConnection | null = null;
  private controlChannel: RTCDataChannel | null = null;
  private sessionChannels = new Map<string, RTCDataChannel>();
  private _status: TransportStatus = 'disconnected';
  private heartbeatInterval: ReturnType<typeof setInterval> | null = null;
  private reconnectTimer: ReturnType<typeof setTimeout> | null = null;
  private reconnectAttempt = 0;
  private intentionalDisconnect = false;
  // Pending request/response pairs for control channel RPC
  private pendingRequests = new Map<string, {
    resolve: (value: any) => void;
    reject: (reason: any) => void;
    timer: ReturnType<typeof setTimeout>;
  }>();
  private requestIdCounter = 0;
  // In-progress gzip chunked transfers (keyed by request_id)
  private gzipTransfers = new Map<string, { chunks: string[]; status: number; expectedChunks: number }>();
  // Sessions requested before connection was ready — replayed on connect
  private pendingSubscriptions = new Set<string>();
  // Abort controller for in-flight signaling (cancelled on cleanup/disconnect)
  private signalingAbort: AbortController | null = null;

  constructor(callbacks: TransportCallbacks, config: RtcTransportConfig = {}) {
    this.callbacks = callbacks;
    this.config = config;
  }

  // --- Transport interface ---

  connect(): void {
    this.intentionalDisconnect = false;
    this.doConnect();
  }

  disconnect(): void {
    this.intentionalDisconnect = true;
    this.cleanup();
    this.setStatus('disconnected');
  }

  subscribeSession(sessionId: string): void {
    if (this.sessionChannels.has(sessionId)) return;
    if (!this.pc || this.pc.connectionState !== 'connected') {
      // Queue — will be replayed when connection establishes
      this.pendingSubscriptions.add(sessionId);
      return;
    }
    this.pendingSubscriptions.delete(sessionId);
    this.openSessionChannel(sessionId);
  }

  unsubscribeSession(sessionId: string): void {
    const dc = this.sessionChannels.get(sessionId);
    if (dc) {
      dc.close();
      this.sessionChannels.delete(sessionId);
    }
  }

  status(): TransportStatus {
    return this._status;
  }

  // --- Outbound (via control channel RPC or session data channel) ---

  async sendChat(req: ChatRequest): Promise<{ session_id?: string; status?: string }> {
    // Always use control channel RPC for chat messages — we need the response
    // to detect { status: "queued" } and remove the optimistic user message.
    return this.controlRequest({ type: 'chat', ...req });
  }

  async sendAskUserResponse(req: AskUserResponse): Promise<void> {
    // Always use control channel RPC so failures are surfaced to the caller.
    await this.controlRequest({ type: 'ask_user_response', ...req });
  }

  async sendPlanAction(req: PlanAction): Promise<void> {
    const { type, ...rest } = req;
    // Always use control channel RPC so failures are surfaced to the caller.
    await this.controlRequest({ type: `plan_${type}`, ...rest });
  }

  async sendClear(projectRoot: string, sessionId?: string | null): Promise<void> {
    await this.controlRequest({ type: 'clear', project_root: projectRoot, session_id: sessionId });
  }

  async sendCompact(projectRoot: string, sessionId: string | null, agentId: string, focus?: string): Promise<{ compacted?: boolean; referenced_files?: string[] }> {
    return this.controlRequest({ type: 'compact', project_root: projectRoot, session_id: sessionId, agent_id: agentId, focus });
  }

  async httpProxy(method: string, url: string, body?: any): Promise<{ status: number; body: string }> {
    return this.controlRequest({ type: 'http_request', method, url, body });
  }

  // --- Internal: connection ---

  private async doConnect(): Promise<void> {
    if (this._status === 'connecting') return; // already in progress
    this.setStatus('connecting');

    try {
      const iceServers = this.config.iceServers || DEFAULT_ICE_SERVERS;
      this.pc = new RTCPeerConnection({ iceServers });

      // Create control channel before offer (so it's in the SDP)
      this.controlChannel = this.pc.createDataChannel('control', { ordered: true });
      this.setupControlChannel(this.controlChannel);

      // Listen for server-created data channels (session channels)
      this.pc.ondatachannel = (event) => {
        const dc = event.channel;
        if (dc.label.startsWith('sess-')) {
          const sessionId = dc.label.slice(5);
          this.setupSessionChannel(sessionId, dc);
        }
      };

      // Connection state tracking — wait for 'failed' only.
      // 'connected' is handled in control channel onopen (below) to ensure
      // the data channel is ready before we start routing API calls.
      this.pc.onconnectionstatechange = () => {
        const state = this.pc?.connectionState;
        if (state === 'failed') {
          // Only disconnect on 'failed', not 'disconnected' (which is transient)
          this.handleDisconnect();
        }
      };

      this.pc.oniceconnectionstatechange = () => {
        if (this.pc?.iceConnectionState === 'failed') {
          this.handleDisconnect();
        }
      };

      // Create offer, gather all ICE candidates (full ICE, no trickle)
      const offer = await this.pc.createOffer();
      await this.pc.setLocalDescription(offer);
      await this.waitForIceGathering();

      // Exchange SDP via the configured signaling strategy
      const signaling = this.config.signaling ?? new WhipSignaling();
      const controller = new AbortController();
      this.signalingAbort = controller;
      const answerSdp = await signaling.exchange(
        this.pc.localDescription!.sdp,
        controller.signal,
      );
      this.signalingAbort = null;
      await this.pc.setRemoteDescription({ type: 'answer', sdp: answerSdp });
    } catch (err) {
      console.error('WebRTC connection failed:', err);
      this.handleDisconnect();
    }
  }

  private waitForIceGathering(): Promise<void> {
    return new Promise((resolve) => {
      if (!this.pc) { resolve(); return; }
      if (this.pc.iceGatheringState === 'complete') { resolve(); return; }

      const timeout = setTimeout(() => {
        // If gathering hasn't completed in 5s, proceed with what we have
        resolve();
      }, 5000);

      this.pc.onicegatheringstatechange = () => {
        if (this.pc?.iceGatheringState === 'complete') {
          clearTimeout(timeout);
          resolve();
        }
      };
    });
  }

  // --- Internal: data channels ---

  private setupControlChannel(dc: RTCDataChannel): void {
    dc.onopen = () => {
      // Control channel is ready — NOW the connection is fully usable.
      // Don't fire these on pc.onconnectionstatechange because the data
      // channel may not be open yet at that point.
      //
      // IMPORTANT: setStatus('connected') MUST come before onReconnect.
      // onReconnect triggers resyncState() which fetches /api/* endpoints.
      // The fetchProxy only routes through WebRTC when status === 'connected',
      // so if we call onReconnect first, all fetches get empty stub responses.
      this.reconnectAttempt = 0;
      this.setStatus('connected');
      this.callbacks.onReconnect?.();
      this.startHeartbeat();
      for (const sid of this.pendingSubscriptions) {
        this.openSessionChannel(sid);
      }
      this.pendingSubscriptions.clear();
    };

    dc.onmessage = (event) => {
      try {
        const msg = JSON.parse(event.data);

        // Handle gzip chunked transfer (large responses like /api/skills)
        if (msg.request_id && msg.gzip_start) {
          this.gzipTransfers.set(msg.request_id, {
            chunks: [],
            status: msg.gzip_start.status || 200,
            expectedChunks: msg.gzip_start.chunks || 0,
          });
          return;
        }
        if (msg.request_id && msg.gzip_chunk) {
          const transfer = this.gzipTransfers.get(msg.request_id);
          if (transfer) transfer.chunks.push(msg.gzip_chunk);
          return;
        }
        if (msg.request_id && msg.gzip_end) {
          const transfer = this.gzipTransfers.get(msg.request_id);
          const pending = this.pendingRequests.get(msg.request_id);
          this.gzipTransfers.delete(msg.request_id);
          if (transfer && pending) {
            this.pendingRequests.delete(msg.request_id);
            // Decode base64 chunks → binary → decompress gzip
            const binaryStr = transfer.chunks.map(c => atob(c)).join('');
            const bytes = new Uint8Array(binaryStr.length);
            for (let i = 0; i < binaryStr.length; i++) bytes[i] = binaryStr.charCodeAt(i);
            new Response(new Blob([bytes]).stream().pipeThrough(new DecompressionStream('gzip')))
              .text()
              .then(body => pending.resolve({ status: transfer.status, body }))
              .catch(err => pending.reject(new Error(`Decompress failed: ${err.message}`)));
          }
          return;
        }

        // Handle RPC responses synchronously (callers are awaiting)
        if (msg.request_id && this.pendingRequests.has(msg.request_id)) {
          const pending = this.pendingRequests.get(msg.request_id)!;
          this.pendingRequests.delete(msg.request_id);
          if (msg.error) {
            pending.reject(new Error(msg.error));
          } else {
            pending.resolve(msg.data || {});
          }
          return;
        }

        // Defer event dispatch to avoid blocking the main thread
        // (prevents input lag from long React state updates)
        if (msg.kind) {
          queueMicrotask(() => this.callbacks.onEvent(null, msg as UiEvent));
        }
      } catch (err) {
        console.error('Control channel parse error:', err);
        this.callbacks.onParseError?.();
      }
    };

    dc.onclose = () => {
      this.handleDisconnect();
    };
  }

  private openSessionChannel(sessionId: string): void {
    if (!this.pc) return;
    const dc = this.pc.createDataChannel(`sess-${sessionId}`, { ordered: true });
    this.setupSessionChannel(sessionId, dc);
  }

  private setupSessionChannel(sessionId: string, dc: RTCDataChannel): void {
    this.sessionChannels.set(sessionId, dc);

    dc.onmessage = (event) => {
      try {
        const msg = JSON.parse(event.data) as UiEvent;
        // Defer to avoid blocking keyboard input on the main thread
        queueMicrotask(() => this.callbacks.onEvent(sessionId, msg));
      } catch (err) {
        console.error(`Session channel ${sessionId} parse error:`, err);
        this.callbacks.onParseError?.();
      }
    };

    dc.onclose = () => {
      this.sessionChannels.delete(sessionId);
    };
  }

  // --- Internal: control channel RPC ---

  private controlRequest(msg: Record<string, unknown>): Promise<any> {
    return new Promise((resolve, reject) => {
      if (!this.controlChannel || this.controlChannel.readyState !== 'open') {
        reject(new Error('Control channel not open'));
        return;
      }

      const requestId = `req-${++this.requestIdCounter}`;

      // Timeout after 30s — timer is cancelled on success/failure
      const timer = setTimeout(() => {
        if (this.pendingRequests.has(requestId)) {
          this.pendingRequests.delete(requestId);
          reject(new Error('Control channel request timeout'));
        }
      }, 30000);

      this.pendingRequests.set(requestId, {
        resolve: (v) => { clearTimeout(timer); resolve(v); },
        reject: (r) => { clearTimeout(timer); reject(r); },
        timer,
      });

      this.controlChannel.send(JSON.stringify({ ...msg, request_id: requestId }));
    });
  }

  // --- Internal: heartbeat ---

  private startHeartbeat(): void {
    this.stopHeartbeat();
    this.heartbeatInterval = setInterval(() => {
      if (this.controlChannel?.readyState === 'open') {
        this.controlChannel.send(JSON.stringify({ type: 'heartbeat', ts: Date.now() }));
      }
    }, 15000);
  }

  private stopHeartbeat(): void {
    if (this.heartbeatInterval) {
      clearInterval(this.heartbeatInterval);
      this.heartbeatInterval = null;
    }
  }

  // --- Internal: reconnection ---

  private handleDisconnect(): void {
    if (this.intentionalDisconnect) return;
    if (this._status === 'reconnecting') return; // already handling

    this.cleanup();
    this.setStatus('reconnecting');

    // Exponential backoff: 1s, 2s, 4s, 8s, 16s, 30s max
    const delay = Math.min(1000 * Math.pow(2, this.reconnectAttempt), 30000);
    this.reconnectAttempt++;

    this.reconnectTimer = setTimeout(() => {
      this.doConnect();
    }, delay);
  }

  private cleanup(): void {
    this.stopHeartbeat();

    // Abort any in-flight signaling (e.g. relay polling)
    if (this.signalingAbort) {
      this.signalingAbort.abort();
      this.signalingAbort = null;
    }

    if (this.reconnectTimer) {
      clearTimeout(this.reconnectTimer);
      this.reconnectTimer = null;
    }

    // Reject all pending requests and clear their timers
    for (const [, pending] of this.pendingRequests) {
      clearTimeout(pending.timer);
      pending.reject(new Error('Transport disconnected'));
    }
    this.pendingRequests.clear();
    this.gzipTransfers.clear();

    // Re-queue active sessions for replay after reconnect
    for (const [sid] of this.sessionChannels) {
      this.pendingSubscriptions.add(sid);
    }
    // Close all session channels
    for (const [, dc] of this.sessionChannels) {
      dc.close();
    }
    this.sessionChannels.clear();

    // Close control channel
    if (this.controlChannel) {
      this.controlChannel.close();
      this.controlChannel = null;
    }

    // Close peer connection
    if (this.pc) {
      this.pc.close();
      this.pc = null;
    }
  }

  private setStatus(status: TransportStatus): void {
    this._status = status;
    this.callbacks.onStatusChange(status);
  }
}
