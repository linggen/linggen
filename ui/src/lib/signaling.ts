/**
 * Signaling strategies for WebRTC SDP offer/answer exchange.
 *
 * Local:  WhipSignaling — single POST to /api/rtc/whip (direct to linggen server)
 * Remote: RelaySignaling — POST offer to relay, poll for answer (via linggen.dev)
 */
import { _originalFetch } from './fetchProxy';

/** Abstracts the SDP offer/answer exchange. */
export interface SignalingStrategy {
  exchange(offerSdp: string, signal: AbortSignal): Promise<string>;
}

/** Local signaling via WHIP — single POST, get SDP answer back. */
export class WhipSignaling implements SignalingStrategy {
  constructor(private whipUrl: string = '/api/rtc/whip') {}

  async exchange(offerSdp: string, signal: AbortSignal): Promise<string> {
    const resp = await _originalFetch(this.whipUrl, {
      method: 'POST',
      headers: { 'Content-Type': 'application/sdp' },
      body: offerSdp,
      signal,
    });
    if (!resp.ok) {
      throw new Error(`WHIP failed: ${resp.status} ${resp.statusText}`);
    }
    return resp.text();
  }
}

/** Remote signaling via relay — POST offer, poll for answer. */
export class RelaySignaling implements SignalingStrategy {
  private instanceId: string;
  private relayBaseUrl: string;

  constructor(instanceId: string, relayBaseUrl?: string) {
    this.instanceId = instanceId;
    // In blob iframe context, relative URLs don't work — need absolute origin.
    // Read from injected meta tag, fall back to same origin, fall back to empty.
    this.relayBaseUrl = relayBaseUrl
      ?? document.querySelector('meta[name="linggen-relay-origin"]')?.getAttribute('content')
      ?? (typeof window !== 'undefined' && window.location.protocol !== 'blob:' ? '' : '');
  }

  async exchange(offerSdp: string, signal: AbortSignal): Promise<string> {
    // 1. POST offer to relay
    const offerResp = await _originalFetch(
      `${this.relayBaseUrl}/api/signaling/${this.instanceId}/offer`,
      {
        method: 'POST',
        credentials: 'include',
        headers: { 'Content-Type': 'application/sdp' },
        body: offerSdp,
        signal,
      },
    );
    if (!offerResp.ok) {
      const data = await offerResp.json().catch(() => ({}));
      throw new Error(data.error || `Offer failed: ${offerResp.status}`);
    }
    const { nonce } = await offerResp.json();

    // 2. Poll for answer
    for (let i = 0; i < 60; i++) {
      if (signal.aborted) throw new DOMException('Aborted', 'AbortError');
      await new Promise(r => setTimeout(r, 500));

      const answerResp = await _originalFetch(
        `${this.relayBaseUrl}/api/signaling/${this.instanceId}/answer?nonce=${encodeURIComponent(nonce)}`,
        { credentials: 'include', signal },
      );
      if (answerResp.status === 204) continue;
      if (answerResp.ok) {
        const { sdp } = await answerResp.json();
        if (sdp) return sdp;
      }
    }
    throw new Error('Connection timed out — no answer received');
  }
}
