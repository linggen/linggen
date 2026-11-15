#!/usr/bin/env node
// Prove the baked clips play the same as the runtime Mixamo retarget.
//   (a) the old runtime's retarget of the raw clip in anim-src/ — petMixamo.ts
//       loadMixamoAnimation for .fbx, GLTFLoader + VRMAnimationLoaderPlugin +
//       createVRMAnimationClip for .vrma (the calls PetStage used to make)
//   (b) petBaked.ts decodeBakedClip on public/anim-baked/<name>.bin
// Both are evaluated with three's own keyframe interpolants at 30 evenly spaced
// times plus every original key time; reports the largest angle between the
// two local bone rotations, the largest hips-position distance, and (extra)
// the largest angle between the bones' WORLD rotations, which is what the eye
// sees (local errors add up down an arm). The two trimmed URLs the app really
// uses (clap#1,3.2, thinking#0,2.5) go through trimClip on both sides.
//
//   node scripts/check-baked.mjs                run on public/anim-baked/
//   node scripts/check-baked.mjs --sweep        also re-bake in memory at several error bounds
import fs from 'node:fs';
import zlib from 'node:zlib';
import * as THREE from 'three';
import { importTs, uiPath } from './_nodeEnv.mjs';
import { encodeClip, loadContext, loadSourceClip, sourceNames } from './bake-anim.mjs';

const SAMPLES = 30;
const deg = (r) => (r * 180) / Math.PI;

function evaluate(track, t) {
  return Array.from(track.createInterpolant().evaluate(t));
}

function sampleTimes(a) {
  const ts = Array.from({ length: SAMPLES }, (_, i) => (a.duration * i) / (SAMPLES - 1));
  const keys = new Set();
  a.tracks.forEach((tr) => tr.times.forEach((t) => keys.add(t)));
  return { ts, all: [...new Set([...ts, ...keys])].sort((x, y) => x - y) };
}

function worldPose(vrm, clip, t) {
  for (const tr of clip.tracks) {
    const node = vrm.scene.getObjectByName(tr.name.split('.')[0]);
    const v = evaluate(tr, t);
    if (tr instanceof THREE.QuaternionKeyframeTrack) node.quaternion.fromArray(v);
    else node.position.fromArray(v);
  }
  vrm.scene.updateMatrixWorld(true);
  return clip.tracks.filter((tr) => tr instanceof THREE.QuaternionKeyframeTrack)
    .map((tr) => vrm.scene.getObjectByName(tr.name.split('.')[0]).getWorldQuaternion(new THREE.Quaternion()));
}

/** Largest local/hips/world differences between clip a (runtime) and clip b (baked). */
function compare(a, b, vrm) {
  const res = { local30: 0, localAll: 0, hips: 0, world: 0, timeDiff: 0, problems: [] };
  if (a.tracks.map((t) => t.name).join() !== b.tracks.map((t) => t.name).join()) res.problems.push('track set/order differs');
  if (Math.abs(a.duration - b.duration) > 1e-5) res.problems.push(`duration ${a.duration} vs ${b.duration}`);
  if (res.problems.length) return res;
  const { ts, all } = sampleTimes(a);
  const on30 = new Set(ts);
  a.tracks.forEach((ta, i) => {
    const tb = b.tracks[i];
    if (ta.times.length === tb.times.length) ta.times.forEach((t, k) => { res.timeDiff = Math.max(res.timeDiff, Math.abs(t - tb.times[k])); });
    const isQuat = ta instanceof THREE.QuaternionKeyframeTrack;
    for (const t of all) {
      const va = evaluate(ta, t), vb = evaluate(tb, t);
      if (isQuat) {
        const d = Math.abs(va[0] * vb[0] + va[1] * vb[1] + va[2] * vb[2] + va[3] * vb[3]);
        const ang = deg(2 * Math.acos(Math.min(1, d)));
        res.localAll = Math.max(res.localAll, ang);
        if (on30.has(t)) res.local30 = Math.max(res.local30, ang);
      } else {
        res.hips = Math.max(res.hips, Math.hypot(va[0] - vb[0], va[1] - vb[1], va[2] - vb[2]));
      }
    }
  });
  const rest = new Map();
  vrm.scene.traverse((o) => rest.set(o, [o.position.clone(), o.quaternion.clone()]));
  for (const t of ts) {
    const wa = worldPose(vrm, a, t), wb = worldPose(vrm, b, t);
    wa.forEach((q, i) => { res.world = Math.max(res.world, deg(q.angleTo(wb[i]))); });
  }
  // posing the skeleton moves the hips node, and loadMixamoAnimation reads that
  // node's height for its scale — put the rig back so the next clip loads from rest
  rest.forEach(([p, q], o) => { o.position.copy(p); o.quaternion.copy(q); });
  vrm.scene.updateMatrixWorld(true);
  return res;
}

const TRIMS = { clap: '1,3.2', thinking: '0,2.5' }; // the fragments PetStage / actions.json use

async function main() {
  const ctx = await loadContext();
  const { decodeBakedClip } = await importTs('src/components/yinyue/petBaked.ts');
  const { trimClip } = await importTs('src/components/yinyue/petBaked.ts');
  const dir = uiPath('public/anim-baked');
  const rows = [];
  const runtime = new Map();
  let tot = { orig: 0, baked: 0 }, worst = { local: 0, hips: 0, world: 0 };
  const verdicts = [];
  for (const f of sourceNames()) {
    const name = f.replace(/\.(fbx|vrma)$/, '');
    const a = await loadSourceClip(ctx, f);
    runtime.set(name, a);
    const bytes = fs.readFileSync(`${dir}/${name}.bin`);
    const b = decodeBakedClip(bytes.buffer.slice(bytes.byteOffset, bytes.byteOffset + bytes.byteLength), ctx.vrm);
    const r = compare(a, b, ctx.vrm);
    const orig = fs.statSync(uiPath('anim-src', f)).size;
    tot.orig += orig; tot.baked += bytes.length;
    worst = { local: Math.max(worst.local, r.localAll), hips: Math.max(worst.hips, r.hips), world: Math.max(worst.world, r.world) };
    rows.push(`${name.padEnd(18)} ${String(orig).padStart(9)} ${String(bytes.length).padStart(7)}  ${r.local30.toFixed(3).padStart(7)} ${r.localAll.toFixed(3).padStart(7)} ${r.world.toFixed(3).padStart(7)} ${(r.hips * 1000).toFixed(3).padStart(8)}  ${r.timeDiff.toExponential(1)} ${r.problems.join('; ')}`);
    if (TRIMS[name]) {
      const rt = compare(trimClip(a, TRIMS[name]), trimClip(b, TRIMS[name]), ctx.vrm);
      if (rt.problems.length) process.exitCode = 1;
      verdicts.push(`trimClip(${name}#${TRIMS[name]}): ${rt.problems.join('; ') || `dur ok, local ${rt.localAll.toFixed(3)} deg, world ${rt.world.toFixed(3)} deg, hips ${(rt.hips * 1000).toFixed(3)} mm`}`);
    }
  }
  console.log('clip                src B  baked B  loc@30° loc@all° world°  hips mm  max|dt| s');
  console.log(rows.join('\n'));
  console.log(`TOTAL ${tot.orig} -> ${tot.baked} bytes (${(tot.orig / tot.baked).toFixed(0)}x)`);
  console.log(`WORST local ${worst.local.toFixed(3)} deg, world ${worst.world.toFixed(3)} deg, hips ${(worst.hips * 1000).toFixed(3)} mm (target < 0.5 deg, < 1 mm)`);
  const vrmaBytes = [...runtime.keys()].filter((n) => n.startsWith('VRMA_')).reduce((n, k) => n + fs.statSync(`${dir}/${k}.bin`).size, 0);
  console.log(`the seven .vrma baked: ${vrmaBytes} bytes (source ${[...runtime.keys()].filter((n) => n.startsWith('VRMA_')).reduce((n, k) => n + fs.statSync(uiPath('anim-src', k + '.vrma')).size, 0)})`);
  verdicts.forEach((v) => console.log(v));
  if (process.argv.includes('--sweep')) await sweep(ctx, runtime, decodeBakedClip);
  if (worst.local >= 0.5 || worst.hips >= 0.001) process.exitCode = 1;
}

async function sweep(ctx, runtime, decodeBakedClip) {
  console.log('\nreduction bound deg | total bytes | gzip -9 bytes | worst local deg | worst world deg | worst hips mm');
  for (const deg of [0, 0.05, 0.1, 0.25, 0.5, 1]) {
    let total = 0, gz = 0, wl = 0, ww = 0, wh = 0;
    for (const [name, a] of runtime) {
      const bytes = encodeClip(a, ctx, { deg });
      total += bytes.length; gz += zlib.gzipSync(bytes, { level: 9 }).length;
      const r = compare(a, decodeBakedClip(bytes.buffer, ctx.vrm), ctx.vrm);
      wl = Math.max(wl, r.localAll); ww = Math.max(ww, r.world); wh = Math.max(wh, r.hips);
    }
    console.log(`${String(deg).padStart(18)} | ${String(total).padStart(11)} | ${String(gz).padStart(13)} | ${wl.toFixed(3).padStart(15)} | ${ww.toFixed(3).padStart(15)} | ${(wh * 1000).toFixed(3)}`);
  }
}

await main();
