// node --test ui/tests/*.test.mts (Node strips the types).
// The offer goes once candidates fall quiet — not at the browser's late
// 'complete' — and a remote or TURN dial first waits for the candidate it
// exists for.
import { test, mock, beforeEach, afterEach } from 'node:test';
import assert from 'node:assert/strict';
import {
  GATHER_CAP_MS, GATHER_IDLE_MS, gatherNeed, waitForIceGathering, type GatheringPeer, type GatherResult,
} from '../src/lib/iceGather.mts';

/** A fake RTCPeerConnection: emits candidates and the gathering state. */
type Handler = ((ev: unknown) => void) | null;
class FakePeer {
  iceGatheringState = 'gathering';
  onicecandidate: Handler = null;
  onicegatheringstatechange: Handler = null;
  emit(type: string) {
    this.onicecandidate?.({ candidate: { candidate: `candidate:1 1 udp 1 10.0.0.1 5000 typ ${type}` } });
  }
  complete() {
    this.iceGatheringState = 'complete';
    this.onicegatheringstatechange?.({});
  }
}
const wait = (pc: FakePeer, opts?: Parameters<typeof waitForIceGathering>[1]) =>
  waitForIceGathering(pc as unknown as GatheringPeer, opts);

/** Whether the wait has resolved yet, and with what. */
function track(p: Promise<GatherResult>) {
  const t: { result: GatherResult | null } = { result: null };
  p.then((r) => { t.result = r; });
  return t;
}
const flush = () => new Promise((r) => setImmediate(r));

beforeEach(() => mock.timers.enable({ apis: ['setTimeout', 'Date'] }));
afterEach(() => mock.timers.reset());

test('local: the offer goes once candidates fall quiet, long before complete', async () => {
  const pc = new FakePeer();
  const t = track(wait(pc));
  pc.emit('host');
  mock.timers.tick(100);
  pc.emit('host');
  mock.timers.tick(GATHER_IDLE_MS - 1);
  await flush();
  assert.equal(t.result, null, 'a new candidate restarts the quiet');
  mock.timers.tick(1);
  await flush();
  assert.deepEqual(t.result, { end: 'idle', candidates: 2, ms: 100 + GATHER_IDLE_MS });
  assert.equal(pc.onicecandidate, null, 'handlers are released');
});

test('no quiet before the first candidate', async () => {
  const pc = new FakePeer();
  const t = track(wait(pc));
  mock.timers.tick(GATHER_IDLE_MS * 4);
  await flush();
  assert.equal(t.result, null);
});

test('the cap ends a wait whose candidates never fall quiet', async () => {
  const pc = new FakePeer();
  const t = track(wait(pc));
  for (let ms = 0; ms < GATHER_CAP_MS; ms += 100) {
    pc.emit('host');
    mock.timers.tick(100);
  }
  await flush();
  assert.equal(t.result?.end, 'cap');
  assert.equal(t.result?.ms, GATHER_CAP_MS);
});

test('the cap ends a wait that saw no candidate at all', async () => {
  const pc = new FakePeer();
  const t = track(wait(pc));
  mock.timers.tick(GATHER_CAP_MS);
  await flush();
  assert.deepEqual(t.result, { end: 'cap', candidates: 0, ms: GATHER_CAP_MS });
});

test('complete ends the wait at once, as does the null candidate', async () => {
  const a = new FakePeer();
  const ta = track(wait(a));
  a.emit('host');
  a.complete();
  await flush();
  assert.equal(ta.result?.end, 'complete');

  const b = new FakePeer();
  const tb = track(wait(b));
  b.onicecandidate?.({ candidate: null });
  await flush();
  assert.equal(tb.result?.end, 'complete');

  const c = new FakePeer();
  c.iceGatheringState = 'complete';
  const tc = track(wait(c));
  await flush();
  assert.equal(tc.result?.end, 'complete');
});

test('remote: host candidates falling quiet do not send the offer; the first srflx does', async () => {
  const pc = new FakePeer();
  const t = track(wait(pc, { need: 'public' }));
  pc.emit('host');
  pc.emit('host');
  mock.timers.tick(1500);
  await flush();
  assert.equal(t.result, null, 'still waiting for a public candidate');
  pc.emit('srflx');
  mock.timers.tick(GATHER_IDLE_MS);
  await flush();
  assert.deepEqual(t.result, { end: 'idle', candidates: 3, ms: 1500 + GATHER_IDLE_MS });
});

test('remote: a relay candidate counts as public', async () => {
  const pc = new FakePeer();
  const t = track(wait(pc, { need: 'public' }));
  pc.emit('host');
  mock.timers.tick(800);
  pc.emit('relay');
  mock.timers.tick(GATHER_IDLE_MS);
  await flush();
  assert.equal(t.result?.end, 'idle');
});

test('TURN: srflx does not release the offer; the relay candidate does', async () => {
  const pc = new FakePeer();
  const t = track(wait(pc, { need: 'relay' }));
  pc.emit('host');
  pc.emit('srflx');
  mock.timers.tick(2000);
  await flush();
  assert.equal(t.result, null);
  pc.emit('relay');
  mock.timers.tick(GATHER_IDLE_MS);
  await flush();
  assert.deepEqual(t.result, { end: 'idle', candidates: 3, ms: 2000 + GATHER_IDLE_MS });
});

test('remote without its public candidate still goes at the cap', async () => {
  const pc = new FakePeer();
  const t = track(wait(pc, { need: 'public' }));
  pc.emit('host');
  mock.timers.tick(GATHER_CAP_MS);
  await flush();
  assert.equal(t.result?.end, 'cap');
});

test('what a dial needs: TURN → relay, remote → public, else any', () => {
  const stun = [{ urls: 'stun:stun.l.google.com:19302' }];
  const turn = [...stun, { urls: ['turns:turn.example.com:443?transport=tcp'], username: 'u', credential: 'c' }];
  assert.equal(gatherNeed(stun, false), 'any');
  assert.equal(gatherNeed(stun, true), 'public');
  assert.equal(gatherNeed(turn, false), 'relay');
  assert.equal(gatherNeed(turn, true), 'relay');
});
