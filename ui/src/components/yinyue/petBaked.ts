/**
 * Loader for Yinyue's animation clips — every Mixamo FBX and pixiv VRMA she
 * plays was baked offline (`scripts/bake-anim.mjs`, run by `npm run build`)
 * into the already-retargeted VRM humanoid tracks, packed small. This is the
 * only clip format the program ships and reads: no raw animation file is
 * served. Returns a THREE.AnimationClip ready for the avatar's AnimationMixer.
 *
 * File layout (little-endian; "varint" = LEB128):
 *   u8 version(=1) · f64 duration · f64 step · varint trackCount
 *   per track:
 *     u8 boneIndex (into BAKED_BONES)
 *     u8 flags (bit0 position, bit1 raw times, bit2 position has x and z — else y only)
 *     varint n · [position: f32 min, f32 max per stored axis]
 *     n times, varint, first absolute then deltas
 *     n values: quaternion = u32 "smallest three" (2-bit index of the dropped
 *               component, then three 10-bit fields), position = u16 per axis between min/max
 * A dense track (flag bit1 clear) was uniformly sampled every `step` seconds
 * from 0: its times are frame numbers and the loader re-expands it to every
 * frame (slerp / lerp between the kept keys), so the clip has the keys the
 * source clip has and `trimClip` cuts it exactly as before. A raw track keeps
 * its own times, in ticks of 1/300 s.
 *
 * Baked for yinyue.vrm, from its rest pose: the hips height scale (and a VRM0
 * flip, not needed for her VRM1 model) are already applied, so playback never
 * depends on what pose she is in when a clip loads — rebake if the model changes.
 * Erasable TypeScript only, so Node can run it for the equality check.
 */
import * as THREE from 'three';
import type { VRM } from '@pixiv/three-vrm';

/** Humanoid bones in the order of petMixamo's rig map (the bake reads that map); a baked track names its bone by index. */
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
const FLAG_XZ = 4;
const SQRT1_2 = Math.SQRT1_2;

class Reader {
  private pos = 0;
  private view: DataView;
  constructor(buf: ArrayBuffer) { this.view = new DataView(buf); }
  u8(): number { return this.view.getUint8(this.pos++); }
  u16(): number { const v = this.view.getUint16(this.pos, true); this.pos += 2; return v; }
  u32(): number { const v = this.view.getUint32(this.pos, true); this.pos += 4; return v; }
  f32(): number { const v = this.view.getFloat32(this.pos, true); this.pos += 4; return v; }
  f64(): number { const v = this.view.getFloat64(this.pos, true); this.pos += 8; return v; }
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

interface PackedTrack { bone: number; stride: number; quat: boolean; raw: boolean; frames: number[]; values: number[] }

function readTrack(r: Reader): PackedTrack {
  const bone = r.u8();
  const flags = r.u8();
  const quat = (flags & FLAG_POSITION) === 0;
  const stride = quat ? 4 : (flags & FLAG_XZ) !== 0 ? 3 : 1;
  const n = r.varint();
  const range: number[][] = [];
  if (!quat) for (let c = 0; c < stride; c++) range.push([r.f32(), r.f32()]);
  const frames: number[] = [];
  let t = 0;
  for (let i = 0; i < n; i++) { t += r.varint(); frames.push(t); }
  const values: number[] = [];
  for (let i = 0; i < n; i++) {
    if (quat) unpackQuat(r.u32(), values, i * 4);
    else for (let c = 0; c < stride; c++) values.push(range[c][0] + (r.u16() / 65535) * (range[c][1] - range[c][0]));
  }
  return { bone, stride, quat, raw: (flags & FLAG_RAW) !== 0, frames, values };
}

/** Put a dense track back on every frame between its kept keys. */
function expandDense(tr: PackedTrack): { frames: number[]; values: number[] } {
  const { stride, values: kept } = tr;
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
        for (let k = 0; k < stride; k++) values.push(kept[j * stride + k]);
      } else if (tr.quat) {
        for (let k = 0; k < 4; k++) values.push(0);
        THREE.Quaternion.slerpFlat(values, at, kept, j * 4, kept, (j + 1) * 4, (f - f0) / (f1 - f0));
      } else {
        const a = (f - f0) / (f1 - f0);
        for (let k = 0; k < stride; k++) values.push(kept[j * stride + k] + (kept[(j + 1) * stride + k] - kept[j * stride + k]) * a);
      }
    }
  }
  return { frames, values };
}

/** Position values for the clip: the stored axes, with x and z zero when only y was kept. */
function positionValues(tr: PackedTrack, values: number[]): number[] {
  return tr.stride === 3 ? values : values.flatMap((y) => [0, y, 0]);
}

/** Decode a baked file into the clip the source file would have produced for `vrm`. */
export function decodeBakedClip(buf: ArrayBuffer, vrm: VRM): THREE.AnimationClip {
  const r = new Reader(buf);
  const version = r.u8();
  if (version !== 1) throw new Error(`baked clip version ${version} unsupported`);
  const duration = r.f64();
  const step = r.f64();
  const count = r.varint();
  const tracks: THREE.KeyframeTrack[] = [];
  for (let i = 0; i < count; i++) {
    const tr = readTrack(r);
    const node = vrm.humanoid?.getNormalizedBoneNode(BAKED_BONES[tr.bone] as never);
    if (!node) continue;
    const dense = tr.raw ? { frames: tr.frames, values: tr.values } : expandDense(tr);
    const unit = tr.raw ? 1 / TICKS_PER_SECOND : step;
    const times = dense.frames.map((f) => f * unit);
    if (tr.quat) tracks.push(new THREE.QuaternionKeyframeTrack(`${node.name}.quaternion`, times, dense.values));
    else tracks.push(new THREE.VectorKeyframeTrack(`${node.name}.position`, times, positionValues(tr, dense.values)));
  }
  return new THREE.AnimationClip('baked', duration, tracks);
}

/** The baked file for a source clip URL: `/anim/idle2.fbx` and `/anim/VRMA_03.vrma` live at `/anim-baked/*.bin`. */
export function bakedUrl(src: string): string {
  return src.replace('/anim/', '/anim-baked/').replace(/\.(fbx|vrma)$/, '.bin');
}

/** Trim a clip to a `start,end` (seconds) window — e.g. to drop a clip's
 *  tail (a Mixamo "Clapping" that ends by sitting down). Empty side = clip
 *  bound. fps is inferred from keyframe spacing so the cut matches seconds. */
export function trimClip(clip: THREE.AnimationClip, frag: string): THREE.AnimationClip {
  const [sStr, eStr] = frag.split(',');
  const start = sStr ? parseFloat(sStr) : 0;
  const end = eStr ? parseFloat(eStr) : clip.duration;
  if (!(end > start)) return clip;
  const t0 = clip.tracks[0];
  const dt = t0 && t0.times.length > 1 ? t0.times[1] - t0.times[0] : 1 / 30;
  const fps = dt > 0 ? Math.round(1 / dt) : 30;
  return THREE.AnimationUtils.subclip(clip, clip.name, Math.round(start * fps), Math.round(end * fps), fps);
}

export async function loadBakedAnimation(url: string, vrm: VRM): Promise<THREE.AnimationClip | null> {
  const res = await fetch(url);
  if (!res.ok) {
    console.warn(`[pet] baked clip ${url}: ${res.status}`);
    return null;
  }
  return decodeBakedClip(await res.arrayBuffer(), vrm);
}
