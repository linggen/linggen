#!/usr/bin/env node
// Bake every public/anim/*.fbx into public/anim-baked/<name>.bin.
//
// The retarget is NOT re-implemented: the app's own petMixamo.ts
// `loadMixamoAnimation` runs here under plain Node (FBXLoader reads from disk,
// yinyue.vrm is loaded with its textures dropped), and its output tracks —
// already on the VRM humanoid — are what get packed. Format: see petBaked.ts.
// Packing = per-track keyframe reduction within an angular error bound, then
// quaternions as 32-bit "smallest three" (10 bits per field), hips height as
// u16, times as varint deltas.
//
//   node scripts/bake-anim.mjs [--deg 0.1] [--pos 0.0003] [--out public/anim-baked]
//
// Generated files are NOT committed (public/anim-baked/ is git-ignored).
import fs from 'node:fs';
import { importTs, loadVrmNode, patchFbxLoaderToDisk, uiPath } from './_nodeEnv.mjs';
import * as THREE from 'three';

const TPS = 300; // ticks per second, must match petBaked.ts
const SQRT1_2 = Math.SQRT1_2;
export const DEFAULTS = { deg: 0.1, pos: 0.0003 };

// ---------- byte writer ----------
class Writer {
  bytes = [];
  u8(v) { this.bytes.push(v & 255); }
  u16(v) { this.u8(v); this.u8(v >> 8); }
  u32(v) { for (let i = 0; i < 4; i++) this.u8(v >>> (8 * i)); }
  f32(v) { const b = Buffer.alloc(4); b.writeFloatLE(v); this.bytes.push(...b); }
  varint(v) {
    while (v >= 0x80) { this.u8((v % 0x80) | 0x80); v = Math.floor(v / 0x80); }
    this.u8(v);
  }
  done() { return Uint8Array.from(this.bytes); }
}

// ---------- quaternion packing (smallest three, 10 bits) ----------
function packQuat(q, at) {
  let drop = 0;
  for (let k = 1; k < 4; k++) if (Math.abs(q[at + k]) > Math.abs(q[at + drop])) drop = k;
  const sign = q[at + drop] < 0 ? -1 : 1;
  let word = drop << 30;
  for (let k = 0, shift = 20; k < 4; k++) {
    if (k === drop) continue;
    const field = Math.round(((sign * q[at + k]) / SQRT1_2 + 1) / 2 * 1023);
    word |= Math.min(1023, Math.max(0, field)) << shift;
    shift -= 10;
  }
  return word >>> 0;
}

function unpackQuat(word, out, at) {
  const drop = word >>> 30;
  const c = [0, 0, 0, 0];
  let sum = 0;
  for (let k = 0, shift = 20; k < 4; k++) {
    if (k === drop) continue;
    c[k] = (((word >>> shift) & 1023) / 1023 * 2 - 1) * SQRT1_2;
    sum += c[k] * c[k];
    shift -= 10;
  }
  c[drop] = Math.sqrt(Math.max(0, 1 - sum));
  for (let k = 0; k < 4; k++) out[at + k] = c[k];
}

const angleDeg = (a, ai, b, bi) => {
  const d = Math.abs(a[ai] * b[bi] + a[ai + 1] * b[bi + 1] + a[ai + 2] * b[bi + 2] + a[ai + 3] * b[bi + 3]);
  return (2 * Math.acos(Math.min(1, d)) * 180) / Math.PI;
};

// ---------- keyframe reduction ----------
// Greedy: from the last kept key, extend to the farthest key such that every
// dropped key in between is within `eps` of the interpolation of the two kept
// ones (kept keys use their dequantised values, so quantisation is included).
function reduceKeys(n, t, errAt) {
  const keep = [0];
  let a = 0;
  while (a < n - 1) {
    let best = a + 1;
    for (let b = a + 2; b < n; b++) {
      let ok = true;
      for (let k = a + 1; k < b && ok; k++) ok = errAt(a, b, k, (t[k] - t[a]) / (t[b] - t[a]));
      if (!ok) break;
      best = b;
    }
    keep.push(best);
    a = best;
  }
  return keep;
}

function quantTrack(values, n) {
  const dq = new Array(n * 4);
  const words = [];
  for (let i = 0; i < n; i++) {
    const w = packQuat(values, i * 4);
    words.push(w);
    unpackQuat(w, dq, i * 4);
  }
  return { dq, words };
}

function keepQuat(track, n, degBound) {
  const { dq, words } = quantTrack(track.values, n);
  const tmp = [0, 0, 0, 0];
  const errOk = (a, b, k, f) => {
    THREE.Quaternion.slerpFlat(tmp, 0, dq, a * 4, dq, b * 4, f);
    return angleDeg(tmp, 0, track.values, k * 4) <= degBound;
  };
  return { keep: reduceKeys(n, track.times, errOk), words };
}

function keepPos(ys, t, posBound, lo, hi) {
  const q = ys.map((y) => (hi > lo ? lo + Math.round(((y - lo) / (hi - lo)) * 65535) / 65535 * (hi - lo) : lo));
  const errOk = (a, b, k, f) => Math.abs(q[a] + (q[b] - q[a]) * f - ys[k]) <= posBound;
  return reduceKeys(ys.length, t, errOk);
}

// ---------- one clip ----------
function classify(clip) {
  // grid step (ticks) of the first track with >= 2 keys; a track is "dense" if it sits on that grid from 0
  const first = clip.tracks.find((tr) => tr.times.length > 1);
  const grid = first ? Math.round((first.times[1] - first.times[0]) * TPS) : 10;
  const isDense = (tr) => tr.times.length > 1 && tr.times.every((tm, i) => Math.abs(tm * TPS - i * grid) < 0.05);
  return { grid, isDense };
}

function encodeTrack(w, tr, boneIdx, dense, grid, opts) {
  const n = tr.times.length;
  const isPos = tr instanceof THREE.VectorKeyframeTrack;
  const unit = dense ? grid / TPS : 1 / TPS;
  const frames = Array.from(tr.times, (tm) => Math.round(tm / unit));
  const maxTimeErr = Math.max(...frames.map((f, i) => Math.abs(f * unit - tr.times[i])));
  if (maxTimeErr > 1e-5) throw new Error(`${tr.name}: times not on the 1/${TPS} s grid (${maxTimeErr})`);
  let ys = null, lo = 0, hi = 0, keep, words;
  if (isPos) {
    ys = Array.from({ length: n }, (_, i) => tr.values[i * 3 + 1]);
    for (let i = 0; i < n; i++) if (tr.values[i * 3] !== 0 || tr.values[i * 3 + 2] !== 0) throw new Error(`${tr.name}: x/z not zero`);
    lo = Math.fround(Math.min(...ys)); hi = Math.fround(Math.max(...ys));
    keep = dense ? keepPos(ys, tr.times, opts.pos, lo, hi) : ys.map((_, i) => i);
  } else if (dense) {
    ({ keep, words } = keepQuat(tr, n, opts.deg));
  } else {
    keep = Array.from({ length: n }, (_, i) => i);
  }
  if (!isPos && !words) words = quantTrack(tr.values, n).words;
  w.u8(boneIdx);
  w.u8((isPos ? 1 : 0) | (dense ? 0 : 2));
  w.varint(keep.length);
  if (isPos) { w.f32(lo); w.f32(hi); }
  let prev = 0;
  for (const k of keep) { w.varint(frames[k] - prev); prev = frames[k]; }
  for (const k of keep) {
    if (isPos) w.u16(hi > lo ? Math.round(((ys[k] - lo) / (hi - lo)) * 65535) : 0);
    else w.u32(words[k]);
  }
}

/** Pack a retargeted clip (as loadMixamoAnimation returns it). `ctx` = { vrm, BAKED_BONES }. */
export function encodeClip(clip, ctx, opts = DEFAULTS) {
  const o = { ...DEFAULTS, ...opts };
  const nodeToBone = new Map();
  ctx.BAKED_BONES.forEach((b, i) => {
    const node = ctx.vrm.humanoid.getNormalizedBoneNode(b);
    if (node) nodeToBone.set(node.name, i);
  });
  const { grid, isDense } = classify(clip);
  const durTicks = Math.round(clip.duration * TPS);
  if (Math.abs(durTicks / TPS - clip.duration) > 1e-5) throw new Error('duration off the tick grid');
  const w = new Writer();
  w.u8(1); w.varint(grid); w.varint(durTicks); w.varint(clip.tracks.length);
  for (const tr of clip.tracks) {
    const bone = nodeToBone.get(tr.name.split('.')[0]);
    if (bone === undefined) throw new Error(`no baked bone for ${tr.name}`);
    encodeTrack(w, tr, bone, isDense(tr), grid, o);
  }
  return w.done();
}

export async function loadContext() {
  patchFbxLoaderToDisk();
  const { loadMixamoAnimation } = await importTs('src/components/yinyue/petMixamo.ts');
  const { BAKED_BONES } = await importTs('src/components/yinyue/petBaked.ts');
  const vrm = await loadVrmNode(uiPath('public/yinyue.vrm'));
  return { loadMixamoAnimation, BAKED_BONES, vrm };
}

export const fbxNames = () => fs.readdirSync(uiPath('public/anim')).filter((f) => f.endsWith('.fbx')).sort();

async function main() {
  const arg = (k, d) => { const i = process.argv.indexOf(`--${k}`); return i > 0 ? process.argv[i + 1] : d; };
  const opts = { deg: Number(arg('deg', DEFAULTS.deg)), pos: Number(arg('pos', DEFAULTS.pos)) };
  const out = uiPath(arg('out', 'public/anim-baked'));
  fs.mkdirSync(out, { recursive: true });
  const ctx = await loadContext();
  let before = 0, after = 0;
  for (const f of fbxNames()) {
    const clip = await ctx.loadMixamoAnimation(`/anim/${f}`, ctx.vrm);
    const bytes = encodeClip(clip, ctx, opts);
    fs.writeFileSync(`${out}/${f.replace(/\.fbx$/, '.bin')}`, bytes);
    const orig = fs.statSync(uiPath('public/anim', f)).size;
    before += orig; after += bytes.length;
    console.log(`${f.padEnd(24)} ${String(orig).padStart(9)} -> ${String(bytes.length).padStart(7)}`);
  }
  console.log(`TOTAL ${before} -> ${after} bytes (deg ${opts.deg}, pos ${opts.pos})`);
}

if (import.meta.url === `file://${process.argv[1]}`) await main();
