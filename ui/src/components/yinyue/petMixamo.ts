/**
 * Load a Mixamo .fbx animation and retarget it onto a VRM humanoid rig at
 * runtime — no Blender, no offline conversion. Adapted from pixiv's official
 * three-vrm example (`loadMixamoAnimation.js`): it remaps the `mixamorig*`
 * bone names to VRM humanoid bones, rebases each rotation into the VRM's rest
 * space, scales the hips translation by the height ratio, and flips signs for
 * VRM0 models (gated on `metaVersion`; Yinyue's own model is VRM1, so no flip).
 *
 * Returns a THREE.AnimationClip ready for the avatar's AnimationMixer.
 */
import * as THREE from 'three';
import { FBXLoader } from 'three/addons/loaders/FBXLoader.js';
import type { VRM } from '@pixiv/three-vrm';

// Mixamo rig bone → VRM humanoid bone name.
const mixamoVRMRigMap: Record<string, string> = {
  mixamorigHips: 'hips',
  mixamorigSpine: 'spine',
  mixamorigSpine1: 'chest',
  mixamorigSpine2: 'upperChest',
  mixamorigNeck: 'neck',
  mixamorigHead: 'head',
  mixamorigLeftShoulder: 'leftShoulder',
  mixamorigLeftArm: 'leftUpperArm',
  mixamorigLeftForeArm: 'leftLowerArm',
  mixamorigLeftHand: 'leftHand',
  mixamorigLeftHandThumb1: 'leftThumbMetacarpal',
  mixamorigLeftHandThumb2: 'leftThumbProximal',
  mixamorigLeftHandThumb3: 'leftThumbDistal',
  mixamorigLeftHandIndex1: 'leftIndexProximal',
  mixamorigLeftHandIndex2: 'leftIndexIntermediate',
  mixamorigLeftHandIndex3: 'leftIndexDistal',
  mixamorigLeftHandMiddle1: 'leftMiddleProximal',
  mixamorigLeftHandMiddle2: 'leftMiddleIntermediate',
  mixamorigLeftHandMiddle3: 'leftMiddleDistal',
  mixamorigLeftHandRing1: 'leftRingProximal',
  mixamorigLeftHandRing2: 'leftRingIntermediate',
  mixamorigLeftHandRing3: 'leftRingDistal',
  mixamorigLeftHandPinky1: 'leftLittleProximal',
  mixamorigLeftHandPinky2: 'leftLittleIntermediate',
  mixamorigLeftHandPinky3: 'leftLittleDistal',
  mixamorigRightShoulder: 'rightShoulder',
  mixamorigRightArm: 'rightUpperArm',
  mixamorigRightForeArm: 'rightLowerArm',
  mixamorigRightHand: 'rightHand',
  mixamorigRightHandThumb1: 'rightThumbMetacarpal',
  mixamorigRightHandThumb2: 'rightThumbProximal',
  mixamorigRightHandThumb3: 'rightThumbDistal',
  mixamorigRightHandIndex1: 'rightIndexProximal',
  mixamorigRightHandIndex2: 'rightIndexIntermediate',
  mixamorigRightHandIndex3: 'rightIndexDistal',
  mixamorigRightHandMiddle1: 'rightMiddleProximal',
  mixamorigRightHandMiddle2: 'rightMiddleIntermediate',
  mixamorigRightHandMiddle3: 'rightMiddleDistal',
  mixamorigRightHandRing1: 'rightRingProximal',
  mixamorigRightHandRing2: 'rightRingIntermediate',
  mixamorigRightHandRing3: 'rightRingDistal',
  mixamorigRightHandPinky1: 'rightLittleProximal',
  mixamorigRightHandPinky2: 'rightLittleIntermediate',
  mixamorigRightHandPinky3: 'rightLittleDistal',
  mixamorigLeftUpLeg: 'leftUpperLeg',
  mixamorigLeftLeg: 'leftLowerLeg',
  mixamorigLeftFoot: 'leftFoot',
  mixamorigLeftToeBase: 'leftToes',
  mixamorigRightUpLeg: 'rightUpperLeg',
  mixamorigRightLeg: 'rightLowerLeg',
  mixamorigRightFoot: 'rightFoot',
  mixamorigRightToeBase: 'rightToes',
};

export async function loadMixamoAnimation(url: string, vrm: VRM): Promise<THREE.AnimationClip | null> {
  const loader = new FBXLoader();
  const asset = await loader.loadAsync(url);

  const clip = THREE.AnimationClip.findByName(asset.animations, 'mixamo.com');
  if (!clip) {
    console.warn(`[pet] no 'mixamo.com' clip in ${url}`);
    return null;
  }

  const tracks: THREE.KeyframeTrack[] = [];
  const restRotationInverse = new THREE.Quaternion();
  const parentRestWorldRotation = new THREE.Quaternion();
  const _quatA = new THREE.Quaternion();
  const _vec3 = new THREE.Vector3();

  // Height ratio (Mixamo rig is ~100× scale and a different height than the VRM).
  const motionHips = asset.getObjectByName('mixamorigHips');
  const vrmHipsNode = vrm.humanoid?.getNormalizedBoneNode('hips');
  if (!motionHips || !vrmHipsNode) return null;
  const motionHipsHeight = motionHips.position.y;
  const vrmHipsY = vrmHipsNode.getWorldPosition(_vec3).y;
  const vrmRootY = vrm.scene.getWorldPosition(_vec3).y;
  const vrmHipsHeight = Math.abs(vrmHipsY - vrmRootY);
  const hipsPositionScale = vrmHipsHeight / motionHipsHeight;

  const isVRM0 = (vrm.meta as { metaVersion?: string } | undefined)?.metaVersion === '0';

  clip.tracks.forEach((track) => {
    const [mixamoRigName, propertyName] = track.name.split('.');
    const vrmBoneName = mixamoVRMRigMap[mixamoRigName];
    const vrmNodeName = vrmBoneName
      ? vrm.humanoid?.getNormalizedBoneNode(vrmBoneName as never)?.name
      : undefined;
    const mixamoRigNode = asset.getObjectByName(mixamoRigName);
    if (!vrmNodeName || !mixamoRigNode || !mixamoRigNode.parent) return;

    mixamoRigNode.getWorldQuaternion(restRotationInverse).invert();
    mixamoRigNode.parent.getWorldQuaternion(parentRestWorldRotation);

    if (track instanceof THREE.QuaternionKeyframeTrack) {
      const values = track.values.slice();
      for (let i = 0; i < values.length; i += 4) {
        _quatA.fromArray(values, i);
        _quatA.premultiply(parentRestWorldRotation).multiply(restRotationInverse);
        _quatA.toArray(values, i);
        if (isVRM0) {
          values[i] = -values[i]; // flip x
          values[i + 2] = -values[i + 2]; // flip z
        }
      }
      tracks.push(new THREE.QuaternionKeyframeTrack(`${vrmNodeName}.${propertyName}`, track.times.slice(), values));
    } else if (track instanceof THREE.VectorKeyframeTrack) {
      // In-place: keep the vertical bob (y), zero horizontal travel (x, z) so a
      // walk/run loops on the spot instead of drifting the docked avatar away.
      const values = track.values.map((v, i) => (i % 3 === 1 ? v * hipsPositionScale : 0));
      tracks.push(new THREE.VectorKeyframeTrack(`${vrmNodeName}.${propertyName}`, track.times.slice(), values));
    }
  });

  return new THREE.AnimationClip('mixamo', clip.duration, tracks);
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
