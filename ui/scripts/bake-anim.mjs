#!/usr/bin/env node
// Bake every raw clip in anim-src/ (Mixamo .fbx, pixiv .vrma) into
// public/anim-baked/<name>.bin — the only animation data the program ships.
//
// Nothing is re-implemented here: the .fbx go through the app's own petMixamo.ts
// `loadMixamoAnimation` (FBXLoader reads from disk, yinyue.vrm is loaded with its
// textures dropped), the .vrma through the same GLTFLoader + VRMAnimationLoaderPlugin +
// createVRMAnimationClip PetStage used to call — all under plain Node, on the rig at
// rest, so the hips scale is fixed by construction. The resulting humanoid tracks are
// packed: per-track keyframe reduction within an angular error bound, quaternions as
// 32-bit "smallest three" (10 bits per field), positions as u16, times as varint
// deltas. The format is documented in src/components/yinyue/petBaked.ts.
//
//   node scripts/bake-anim.mjs [--deg 0.1] [--pos 0.0003] [--out public/anim-baked]
//   node scripts/bake-anim.mjs --build     what `npm run build` runs first: bake what is stale,
//                                          keep existing baked files when there are no sources
//
// Generated files are NOT committed (public/anim-baked/ is git-ignored).
import fs from 'node:fs';
import * as THREE from 'three';
import { GLTFLoader } from 'three/addons/loaders/GLTFLoader.js';
import { VRMAnimationLoaderPlugin, createVRMAnimationClip } from '@pixiv/three-vrm-animation';
import { importTs, loadVrmNode, patchFbxLoaderToDisk, uiPath } from './_nodeEnv.mjs';

const TPS = 300; // ticks per second, must match petBaked.ts
const SQRT1_2 = Math.SQRT1_2;
export const DEFAULTS = { deg: 0.1, pos: 0.0003 };
const SRC_DIR = uiPath('anim-src');
const OUT_DIR = uiPath('public/anim-baked');
const isClip = (f) => /\.(fbx|vrma)$/.test(f);
const bakedName = (f) => f.replace(/\.(fbx|vrma)$/, '.bin');

// ---------- byte writer ----------
class Writer {
  bytes = [];
  u8(v) { this.bytes.push(v & 255); }
  u16(v) { this.u8(v); this.u8(v >> 8); }
  u32(v) { for (let i = 0; i < 4; i++) this.u8(v >>> (8 * i)); }
  f32(v) { const b = Buffer.alloc(4); b.writeFloatLE(v); this.bytes.push(...b); }
  f64(v) { const b = Buffer.alloc(8); b.writeDoubleLE(v); this.bytes.push(...b); }
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
// dropped key in between is within the bound of the interpolation of the two kept
// ones (kept keys use their dequantised values, so quantisation is included).
function reduceKeys(n, t, errOk) {
  const keep = [0];
  let a = 0;
  while (a < n - 1) {
    let best = a + 1;
    for (let b = a + 2; b < n; b++) {
      let ok = true;
      for (let k = a + 1; k < b && ok; k++) ok = errOk(a, b, k, (t[k] - t[a]) / (t[b] - t[a]));
      if (!ok) break;
      best = b;
    }
    keep.push(best);
    a = best;
  }
  return keep;
}

function quantQuats(values, n) {
  const dq = new Array(n * 4);
  const words = [];
  for (let i = 0; i < n; i++) {
    const w = packQuat(values, i * 4);
    words.push(w);
    unpackQuat(w, dq, i * 4);
  }
  return { dq, words };
}

function reduceQuat(tr, n, degBound) {
  const { dq, words } = quantQuats(tr.values, n);
  const tmp = [0, 0, 0, 0];
  const errOk = (a, b, k, f) => {
    THREE.Quaternion.slerpFlat(tmp, 0, dq, a * 4, dq, b * 4, f);
    return angleDeg(tmp, 0, tr.values, k * 4) <= degBound;
  };
  return { keep: reduceKeys(n, tr.times, errOk), words };
}

// positions: per stored axis, u16 between a float32 min and max
function quantPos(values, n, axes) {
  const range = axes.map((ax) => {
    const col = Array.from({ length: n }, (_, i) => values[i * 3 + ax]);
    return [Math.fround(Math.min(...col)), Math.fround(Math.max(...col))];
  });
  const code = (v, [lo, hi]) => (hi > lo ? Math.round(((v - lo) / (hi - lo)) * 65535) : 0);
  const deq = (c, [lo, hi]) => lo + (c / 65535) * (hi - lo);
  const codes = Array.from({ length: n }, (_, i) => axes.map((ax, a) => code(values[i * 3 + ax], range[a])));
  const dq = codes.map((cs) => cs.map((c, a) => deq(c, range[a])));
  return { range, codes, dq };
}

function reducePos(tr, n, posBound, q, axes) {
  const errOk = (a, b, k, f) => {
    let d2 = 0;
    axes.forEach((ax, i) => { const v = q.dq[a][i] + (q.dq[b][i] - q.dq[a][i]) * f - tr.values[k * 3 + ax]; d2 += v * v; });
    return Math.sqrt(d2) <= posBound;
  };
  return reduceKeys(n, tr.times, errOk);
}

// ---------- one clip ----------
/** Uniform sampling step (s) of the first multi-key track; a track is "dense" if it sits on that grid from 0.
 *  A whole frame rate (1/30, 1/60) is used when the times fit it: the loader then rebuilds float32 times
 *  identical to the source's, which matters where trimClip cuts exactly on a key. */
function classify(clip) {
  const first = clip.tracks.find((tr) => tr.times.length > 1);
  const fits = (s) => first.times.every((tm, i) => Math.abs(tm - i * s) < 1e-5);
  let step = 1 / 30;
  if (first) {
    step = (first.times.at(-1) - first.times[0]) / (first.times.length - 1);
    if (fits(1 / Math.round(1 / step))) step = 1 / Math.round(1 / step);
  }
  const isDense = (tr) => tr.times.length > 1 && tr.times.every((tm, i) => Math.abs(tm - i * step) < 1e-5);
  return { step, isDense };
}

function writeKeys(w, frames, keep) {
  let prev = 0;
  for (const k of keep) { w.varint(frames[k] - prev); prev = frames[k]; }
}

function encodeTrack(w, tr, boneIdx, dense, step, opts) {
  const n = tr.times.length;
  const isPos = tr instanceof THREE.VectorKeyframeTrack;
  const unit = dense ? step : 1 / TPS;
  const frames = Array.from(tr.times, (tm) => Math.round(tm / unit));
  const timeErr = Math.max(...frames.map((f, i) => Math.abs(f * unit - tr.times[i])));
  if (timeErr > 1e-5) throw new Error(`${tr.name}: times not on the ${dense ? 'step' : '1/' + TPS + ' s'} grid (${timeErr})`);
  const all = Array.from({ length: n }, (_, i) => i);

  if (!isPos) {
    const { keep, words } = dense ? reduceQuat(tr, n, opts.deg) : { keep: all, words: quantQuats(tr.values, n).words };
    w.u8(boneIdx); w.u8(dense ? 0 : 2); w.varint(keep.length);
    writeKeys(w, frames, keep);
    keep.forEach((k) => w.u32(words[k]));
    return;
  }
  // the FBX retarget zeroes x and z (in-place hips): store y only; a VRMA hips track moves in all three
  const xz = tr.values.some((v, i) => i % 3 !== 1 && v !== 0);
  const axes = xz ? [0, 1, 2] : [1];
  const q = quantPos(tr.values, n, axes);
  const keep = dense ? reducePos(tr, n, opts.pos, q, axes) : all;
  w.u8(boneIdx); w.u8(1 | (dense ? 0 : 2) | (xz ? 4 : 0)); w.varint(keep.length);
  q.range.forEach(([lo, hi]) => { w.f32(lo); w.f32(hi); });
  writeKeys(w, frames, keep);
  keep.forEach((k) => q.codes[k].forEach((c) => w.u16(c)));
}

/** Pack a retargeted clip. `ctx` = { vrm, BAKED_BONES }. */
export function encodeClip(clip, ctx, opts = DEFAULTS) {
  const o = { ...DEFAULTS, ...opts };
  const nodeToBone = new Map();
  ctx.BAKED_BONES.forEach((b, i) => {
    const node = ctx.vrm.humanoid.getNormalizedBoneNode(b);
    if (node) nodeToBone.set(node.name, i);
  });
  const { step, isDense } = classify(clip);
  const w = new Writer();
  w.u8(1); w.f64(clip.duration); w.f64(step); w.varint(clip.tracks.length);
  for (const tr of clip.tracks) {
    const bone = nodeToBone.get(tr.name.split('.')[0]);
    if (bone === undefined) throw new Error(`no baked bone for ${tr.name}`);
    encodeTrack(w, tr, bone, isDense(tr), step, o);
  }
  return w.done();
}

// ---------- sources ----------
export async function loadContext() {
  patchFbxLoaderToDisk();
  const { loadMixamoAnimation } = await importTs('src/components/yinyue/petMixamo.ts');
  const { BAKED_BONES } = await importTs('src/components/yinyue/petBaked.ts');
  const vrm = await loadVrmNode(uiPath('public/yinyue.vrm'));
  return { loadMixamoAnimation, BAKED_BONES, vrm };
}

/** The retargeted clip the old runtime built from `anim-src/<file>` — the very same calls. */
export async function loadSourceClip(ctx, file) {
  if (file.endsWith('.fbx')) return ctx.loadMixamoAnimation(`/anim/${file}`, ctx.vrm);
  const b = fs.readFileSync(`${SRC_DIR}/${file}`);
  const loader = new GLTFLoader();
  loader.register((parser) => new VRMAnimationLoaderPlugin(parser));
  const gltf = await new Promise((res, rej) => loader.parse(b.buffer.slice(b.byteOffset, b.byteOffset + b.byteLength), '', res, rej));
  const anims = gltf.userData.vrmAnimations;
  if (!anims || anims.length === 0) throw new Error(`${file} contains no VRM animation`);
  return createVRMAnimationClip(anims[0], ctx.vrm);
}

export const sourceNames = () => (fs.existsSync(SRC_DIR) ? fs.readdirSync(SRC_DIR).filter(isClip).sort() : []);

// ---------- cli ----------
const mtime = (p) => (fs.existsSync(p) ? fs.statSync(p).mtimeMs : 0);

async function bake(files, opts, out) {
  fs.mkdirSync(out, { recursive: true });
  const ctx = await loadContext();
  let before = 0, after = 0;
  for (const f of files) {
    const clip = await loadSourceClip(ctx, f);
    const bytes = encodeClip(clip, ctx, opts);
    fs.writeFileSync(`${out}/${bakedName(f)}`, bytes);
    const orig = fs.statSync(`${SRC_DIR}/${f}`).size; // follows the symlink
    before += orig; after += bytes.length;
    console.log(`${f.padEnd(24)} ${String(orig).padStart(9)} -> ${String(bytes.length).padStart(7)}`);
  }
  console.log(`baked ${files.length} clips: ${before} -> ${after} bytes (deg ${opts.deg}, pos ${opts.pos})`);
}

/** `npm run build`'s first step. */
async function build() {
  const sources = sourceNames();
  const baked = fs.existsSync(OUT_DIR) ? fs.readdirSync(OUT_DIR).filter((f) => f.endsWith('.bin')) : [];
  if (sources.length === 0) {
    if (baked.length === 0) {
      // The clips are in no repository (their licences do not allow it), so a build off this machine has none.
      // It must say so on purpose: a Mac release built without them would ship her with no gestures.
      if (process.env.LINGGEN_NO_CLIPS === '1') {
        console.warn('bake-anim: LINGGEN_NO_CLIPS=1 — building without her animation clips (she keeps her procedural motions only)');
        return;
      }
      console.error('bake-anim: no animation sources in ui/anim-src/ and no baked clips in ui/public/anim-baked/ — copy the clips into ui/anim-src/ (see scripts/worktree-setup.sh), or set LINGGEN_NO_CLIPS=1 to build without them.');
      process.exit(1);
    }
    console.log(`bake-anim: no ui/anim-src/ — using the ${baked.length} baked clips already in public/anim-baked/`);
    return;
  }
  const vrmTime = mtime(uiPath('public/yinyue.vrm'));
  const stale = sources.filter((f) => mtime(`${OUT_DIR}/${bakedName(f)}`) <= Math.max(fs.statSync(`${SRC_DIR}/${f}`).mtimeMs, vrmTime));
  if (stale.length === 0) {
    console.log(`bake-anim: all ${sources.length} baked clips are newer than their sources — nothing to bake`);
    return;
  }
  await bake(stale, DEFAULTS, OUT_DIR);
}

async function main() {
  if (process.argv.includes('--build')) return build();
  const arg = (k, d) => { const i = process.argv.indexOf(`--${k}`); return i > 0 ? process.argv[i + 1] : d; };
  const opts = { deg: Number(arg('deg', DEFAULTS.deg)), pos: Number(arg('pos', DEFAULTS.pos)) };
  const files = sourceNames();
  if (files.length === 0) { console.error('bake-anim: ui/anim-src/ has no clips'); process.exit(1); }
  await bake(files, opts, uiPath(arg('out', 'public/anim-baked')));
}

if (import.meta.url === `file://${process.argv[1]}`) await main();
