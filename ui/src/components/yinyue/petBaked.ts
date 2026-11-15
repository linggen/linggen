/**
 * Loader for a Mixamo clip that was baked offline (`scripts/bake-anim.mjs`):
 * the already-retargeted VRM humanoid tracks, packed small. Returns the same
 * kind of THREE.AnimationClip `loadMixamoAnimation` returns, so `trimClip` and
 * the mixer treat it identically.
 *
 * File layout (little-endian; "varint" = LEB128):
 *   u8 version(=1) · varint gridTicks · varint durationTicks · varint trackCount
 *   per track:
 *     u8 boneIndex (into BAKED_BONES) · u8 flags (bit0 hips-y position, bit1 raw times)
 *     varint n · [position: f32 min, f32 max]
 *     n times, varint, first absolute then deltas
 *     n values: quaternion = u32 "smallest three" (2-bit index of the dropped
 *               component, then three 10-bit fields), position = u16 between min/max
 * One tick = 1/300 s. A dense track (flag bit1 clear) was uniformly sampled on
 * the clip's grid: its times are grid frames and the loader re-expands it to
 * every frame (slerp / lerp between the kept keys), so the clip has the keys
 * the FBX path has and `trimClip` cuts it exactly as before. A raw track keeps
 * its own times in ticks.
 *
 * Baked for yinyue.vrm: the hips height scale (and a VRM0 flip, not needed for
 * her VRM1 model) are already applied — rebake if the model changes.
 * Erasable TypeScript only, so Node can run it for the equality check.
 */
import * as THREE from 'three';
import type { VRM } from '@pixiv/three-vrm';

/** Humanoid bones in the order of petMixamo's rig map; a baked track names its bone by index. */
export const BAKED_BONES = [
  'hips', 'spine', 'chest', 'upperChest', 'neck', 'head',
  'leftShoulder', 'leftUpperArm', 'leftLowerArm', 'leftHand',
  'leftThumbMetacarpal', 'leftThumbProximal', 'leftThumbDistal',
  'leftIndexProximal', 'leftIndexIntermediate', 'leftIndexDistal',
  'leftMiddleProximal', 'leftMiddleIntermediate', 'leftMiddleDistal',
  'leftRingProximal', 'leftRingIntermediate', 'leftRingDistal',
  'leftLittleProximal', 'leftLittleIntermediate', 'leftLittleDistal',
  'rightShoulder', 'rightUpperArm', 'rightLowerArm', 'rightHand',
  'rightThumbMetacarpal', 'rightThumbProximal', 'rightThumbDistal',
  'rightIndexProximal', 'rightIndexIntermediate', 'rightIndexDistal',
  'rightMiddleProximal', 'rightMiddleIntermediate', 'rightMiddleDistal',
  'rightRingProximal', 'rightRingIntermediate', 'rightRingDistal',
  'rightLittleProximal', 'rightLittleIntermediate', 'rightLittleDistal',
  'leftUpperLeg', 'leftLowerLeg', 'leftFoot', 'leftToes',
  'rightUpperLeg', 'rightLowerLeg', 'rightFoot', 'rightToes',
] as const;

export const TICKS_PER_SECOND = 300;
const FLAG_POSITION = 1;
const FLAG_RAW = 2;
const SQRT1_2 = Math.SQRT1_2;

class Reader {
  private pos = 0;
  private view: DataView;
  constructor(buf: ArrayBuffer) { this.view = new DataView(buf); }
  u8(): number { return this.view.getUint8(this.pos++); }
  u16(): number { const v = this.view.getUint16(this.pos, true); this.pos += 2; return v; }
  u32(): number { const v = this.view.getUint32(this.pos, true); this.pos += 4; return v; }
  f32(): number { const v = this.view.getFloat32(this.pos, true); this.pos += 4; return v; }
  varint(): number {
    let v = 0;
    let shift = 0;
    for (;;) {
      const b = this.u8();
      v += (b & 0x7f) * 2 ** shift;
      if (b < 0x80) return v;
      shift += 7;
    }
  }
}

/** Inverse of the encoder's smallest-three packing → unit quaternion (x,y,z,w) written to `out`. */
function unpackQuat(word: number, out: number[], at: number) {
  const drop = word >>> 30;
  const c = [0, 0, 0, 0];
  let sum = 0;
  for (let k = 0, shift = 20; k < 4; k++) {
    if (k === drop) continue;
    const field = (word >>> shift) & 1023;
    c[k] = (field / 1023 * 2 - 1) * SQRT1_2;
    sum += c[k] * c[k];
    shift -= 10;
  }
  c[drop] = Math.sqrt(Math.max(0, 1 - sum));
  for (let k = 0; k < 4; k++) out[at + k] = c[k];
}

interface RawTrack { bone: number; position: boolean; raw: boolean; frames: number[]; values: number[] }

function readTrack(r: Reader): RawTrack {
  const bone = r.u8();
  const flags = r.u8();
  const position = (flags & FLAG_POSITION) !== 0;
  const n = r.varint();
  const min = position ? r.f32() : 0;
  const max = position ? r.f32() : 0;
  const frames: number[] = [];
  let t = 0;
  for (let i = 0; i < n; i++) { t += r.varint(); frames.push(t); }
  const values: number[] = [];
  for (let i = 0; i < n; i++) {
    if (position) values.push(min + (r.u16() / 65535) * (max - min));
    else unpackQuat(r.u32(), values, i * 4);
  }
  return { bone, position, raw: (flags & FLAG_RAW) !== 0, frames, values };
}

/** Put a dense track back on every grid frame between its kept keys. */
function expandDense(tr: RawTrack): { frames: number[]; values: number[] } {
  const stride = tr.position ? 1 : 4;
  const frames: number[] = [];
  const values: number[] = [];
  const last = tr.frames.length - 1;
  for (let j = 0; j <= last; j++) {
    const f0 = tr.frames[j];
    const f1 = j < last ? tr.frames[j + 1] : f0 + 1;
    for (let f = f0; f < f1; f++) {
      const at = values.length;
      frames.push(f);
      if (j === last || f === f0) {
        for (let k = 0; k < stride; k++) values.push(tr.values[j * stride + k]);
      } else if (tr.position) {
        const a = (f - f0) / (f1 - f0);
        values.push(tr.values[j] + (tr.values[j + 1] - tr.values[j]) * a);
      } else {
        for (let k = 0; k < 4; k++) values.push(0);
        THREE.Quaternion.slerpFlat(values, at, tr.values, j * 4, tr.values, (j + 1) * 4, (f - f0) / (f1 - f0));
      }
    }
  }
  return { frames, values };
}

/** Decode a baked file into the clip the Mixamo path would have produced for `vrm`. */
export function decodeBakedClip(buf: ArrayBuffer, vrm: VRM): THREE.AnimationClip {
  const r = new Reader(buf);
  const version = r.u8();
  if (version !== 1) throw new Error(`baked clip version ${version} unsupported`);
  const grid = r.varint();
  const duration = r.varint() / TICKS_PER_SECOND;
  const count = r.varint();
  const tracks: THREE.KeyframeTrack[] = [];
  for (let i = 0; i < count; i++) {
    const tr = readTrack(r);
    const node = vrm.humanoid?.getNormalizedBoneNode(BAKED_BONES[tr.bone] as never);
    if (!node) continue;
    const dense = tr.raw ? { frames: tr.frames, values: tr.values } : expandDense(tr);
    const unit = tr.raw ? 1 / TICKS_PER_SECOND : grid / TICKS_PER_SECOND;
    const times = dense.frames.map((f) => f * unit);
    if (tr.position) {
      // runtime retarget keeps the vertical bob only (x, z = 0)
      const values = dense.values.flatMap((y) => [0, y, 0]);
      tracks.push(new THREE.VectorKeyframeTrack(`${node.name}.position`, times, values));
    } else {
      tracks.push(new THREE.QuaternionKeyframeTrack(`${node.name}.quaternion`, times, dense.values));
    }
  }
  return new THREE.AnimationClip('mixamo', duration, tracks);
}

export async function loadBakedAnimation(url: string, vrm: VRM): Promise<THREE.AnimationClip | null> {
  const res = await fetch(url);
  if (!res.ok) {
    console.warn(`[pet] baked clip ${url}: ${res.status}`);
    return null;
  }
  return decodeBakedClip(await res.arrayBuffer(), vrm);
}
