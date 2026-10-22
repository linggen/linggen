/**
 * When the offer may go: the wait for ICE candidates before a non-trickle
 * (full ICE) offer.
 *
 * The browser's own "gathering complete" lags far behind the candidates —
 * every candidate is in within ~200 ms, yet `iceGatheringState` sits in
 * 'gathering' for 9–12 s (a STUN server or interface that never answers).
 * So the offer goes once the candidates fall quiet: at least one in hand
 * and none new for `idleMs`. A remote dial also needs a public candidate
 * (srflx or relay) before that quiet counts, and a dial with TURN needs its
 * relay candidate — those can take longer than the quiet. 'complete' and
 * the cap resolve whatever is in hand.
 */

/** The parts of an RTCPeerConnection the wait reads. */
export type GatheringPeer = Pick<RTCPeerConnection, 'iceGatheringState' | 'onicecandidate' | 'onicegatheringstatechange'>;

/** Which candidate must be in hand before the quiet counts. */
export type GatherNeed = 'any' | 'public' | 'relay';

export interface GatherOptions {
  need?: GatherNeed;
  idleMs?: number;
  capMs?: number;
}

export type GatherEnd = 'idle' | 'complete' | 'cap';

export interface GatherResult {
  end: GatherEnd;
  candidates: number;
  ms: number;
}

export const GATHER_IDLE_MS = 250;
export const GATHER_CAP_MS = 5000;

/** `relay` when the ICE servers carry TURN, `public` when the signaling is
 *  remote (the peer is off this machine), else `any`. */
export function gatherNeed(iceServers: RTCIceServer[], remote: boolean): GatherNeed {
  const urls = iceServers.flatMap((s) => (Array.isArray(s.urls) ? s.urls : [s.urls]));
  if (urls.some((u) => /^turns?:/i.test(u))) return 'relay';
  return remote ? 'public' : 'any';
}

/** host | srflx | prflx | relay, from the candidate's `type` or its line. */
export function candidateType(c: { candidate: string; type?: string | null }): string {
  if (c.type) return c.type;
  return / typ (\w+)/.exec(c.candidate)?.[1] ?? '';
}

function satisfies(need: GatherNeed, type: string): boolean {
  if (need === 'any') return true;
  if (need === 'relay') return type === 'relay';
  return type === 'srflx' || type === 'relay';
}

/** Resolves when the offer may go. Call before setLocalDescription so no
 *  candidate fires into a gap; it takes over the peer's two handlers and
 *  clears them when it resolves. */
export function waitForIceGathering(pc: GatheringPeer, opts: GatherOptions = {}): Promise<GatherResult> {
  const need = opts.need ?? 'any';
  const idleMs = opts.idleMs ?? GATHER_IDLE_MS;
  const start = Date.now();
  let candidates = 0;
  let needMet = false;
  let idle: ReturnType<typeof setTimeout> | null = null;

  return new Promise((resolve) => {
    const finish = (end: GatherEnd) => {
      clearTimeout(cap);
      if (idle) clearTimeout(idle);
      pc.onicecandidate = null;
      pc.onicegatheringstatechange = null;
      resolve({ end, candidates, ms: Date.now() - start });
    };
    const cap = setTimeout(() => finish('cap'), opts.capMs ?? GATHER_CAP_MS);

    pc.onicecandidate = (ev) => {
      if (!ev.candidate) { finish('complete'); return; }
      if (!ev.candidate.candidate) return; // end-of-candidates marker
      candidates++;
      if (satisfies(need, candidateType(ev.candidate))) needMet = true;
      if (!needMet) return;
      if (idle) clearTimeout(idle);
      idle = setTimeout(() => finish('idle'), idleMs);
    };
    pc.onicegatheringstatechange = () => {
      if (pc.iceGatheringState === 'complete') finish('complete');
    };
    if (pc.iceGatheringState === 'complete') finish('complete');
  });
}
