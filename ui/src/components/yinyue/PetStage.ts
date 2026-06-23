// PetStage — Yinyue's VRM renderer for the web surface.
//
// Ported from the desktop pet (`linggen-app/shell/pet-ui`) and trimmed to be
// RENDER-ONLY: audio playback + the loudness envelope live in the event
// handler (`lib/eventHandlers/yinyue.ts`, the proven voice path); this engine
// just renders her and takes an external **mouth target** (0..1) each frame so
// lip-sync stays in step with the audio without owning it.
//
// Animation model (layered, procedural — no preset clips yet):
//   - Face = VRM blendshapes (expressionManager): emotion + visemes + blink.
//   - Body = procedural per-frame bone offsets over the captured rest pose.
//   - Gaze = VRM lookAt eyes + subtle head follow.

import * as THREE from 'three';
import { GLTFLoader } from 'three/addons/loaders/GLTFLoader.js';
import { VRMLoaderPlugin, VRMUtils, type VRM, type VRMHumanBoneName } from '@pixiv/three-vrm';
import {
  VRMAnimationLoaderPlugin,
  createVRMAnimationClip,
  type VRMAnimation,
} from '@pixiv/three-vrm-animation';

export type EmotionName = 'neutral' | 'happy' | 'angry' | 'sad' | 'relaxed';
export type ActionName = 'nod' | 'shake' | 'wave' | 'bow' | 'dance' | 'cheer';

const EMOTIONS: Exclude<EmotionName, 'neutral'>[] = ['happy', 'angry', 'sad', 'relaxed'];

// One-shot gesture durations (seconds).
const ACTION_DUR: Record<ActionName, number> = {
  nod: 0.9, shake: 0.9, wave: 1.6, bow: 1.8, dance: 3.0, cheer: 1.6,
};

const MANAGED: VRMHumanBoneName[] = [
  'spine', 'chest', 'upperChest', 'neck', 'head',
  'leftUpperArm', 'rightUpperArm', 'leftLowerArm', 'rightLowerArm',
];

interface Offset { x: number; y: number; z: number }

interface ActiveAction { name: ActionName; t: number; dur: number }

function clamp(v: number, lo: number, hi: number) {
  return Math.max(lo, Math.min(hi, v));
}

export class PetStage {
  private renderer: THREE.WebGLRenderer;
  private scene = new THREE.Scene();
  private camera = new THREE.PerspectiveCamera(28, 1, 0.1, 100);
  private clock = new THREE.Clock();
  private vrm: VRM | null = null;
  private mixer: THREE.AnimationMixer | null = null; // drives .vrma clips
  private currentAction: THREE.AnimationAction | null = null; // playing clip, if any
  private clipCache = new Map<string, THREE.AnimationClip>();
  private raf = 0;
  private resizeObs: ResizeObserver;
  private rest = new Map<VRMHumanBoneName, THREE.Quaternion>();
  private lookTarget = new THREE.Object3D();
  private baseY = 0;
  private gazeY = 1.3;

  // Drivers
  private emotion: EmotionName = 'neutral';
  private emotionWeight = 0;
  private blinkValue = 0;
  private blinkTimer = 2;
  private blinkProgress = -1;
  private mouthValue = 0;
  private mouthTarget = 0; // 0..1, set externally from the audio envelope
  private action: ActiveAction | null = null; // one-shot gesture overlay
  private visible = true; // sustained show/hide target (appear / disappear)
  private visibility = 1; // animated 0..1, damps toward visible ? 1 : 0
  private visApplied = 1; // last opacity/scale written (skip churn once settled)
  private cursor = { x: 0, y: 0 };

  constructor(private canvas: HTMLCanvasElement) {
    this.renderer = new THREE.WebGLRenderer({ canvas, antialias: true, alpha: true });
    this.renderer.setPixelRatio(Math.min(window.devicePixelRatio, 2));
    this.renderer.outputColorSpace = THREE.SRGBColorSpace;

    this.camera.position.set(0, 1.3, 1.4);
    this.scene.add(this.lookTarget);

    const amb = new THREE.AmbientLight(0xffffff, 1.6);
    const dir = new THREE.DirectionalLight(0xffffff, 1.2);
    dir.position.set(0.5, 1.5, 1.2);
    this.scene.add(amb, dir);

    this.resizeObs = new ResizeObserver(() => this.resize());
    this.resizeObs.observe(canvas);
    this.resize();
    this.animate();
  }

  async load(url: string): Promise<void> {
    const loader = new GLTFLoader();
    loader.register((parser) => new VRMLoaderPlugin(parser));
    const gltf = await loader.loadAsync(url);
    const vrm = gltf.userData.vrm as VRM;

    VRMUtils.removeUnnecessaryVertices?.(gltf.scene);
    VRMUtils.combineSkeletons?.(gltf.scene);
    VRMUtils.rotateVRM0?.(vrm);
    vrm.scene.traverse((o) => { o.frustumCulled = false; });

    if (vrm.lookAt) vrm.lookAt.target = this.lookTarget;

    for (const name of MANAGED) {
      const node = vrm.humanoid?.getNormalizedBoneNode(name);
      if (node) this.rest.set(name, node.quaternion.clone());
    }

    this.scene.add(vrm.scene);
    this.frameCamera(vrm);
    this.vrm = vrm;

    this.mixer = new THREE.AnimationMixer(vrm.scene);
    this.mixer.addEventListener('finished', () => {
      // A one-shot clip ended → hand the body back to the procedural layer.
      this.currentAction = null;
    });
  }

  // --- Director API ---

  setEmotion(name: EmotionName) { this.emotion = name; }

  /** Set the mouth opening (0..1) — driven each frame from the audio envelope. */
  setMouthTarget(v: number) { this.mouthTarget = clamp(v, 0, 1); }

  /** Play a one-shot gesture (overlays the idle/procedural layer, then clears). */
  playAction(name: ActionName) {
    this.action = { name, t: 0, dur: ACTION_DUR[name] };
  }

  /** Show or hide the whole avatar (appear / disappear) — eased via damping. */
  setVisible(v: boolean) {
    this.visible = v;
  }

  /** Load + cache a `.vrma`, retargeted to this VRM's humanoid rig. */
  private async loadClip(url: string): Promise<THREE.AnimationClip | null> {
    const cached = this.clipCache.get(url);
    if (cached) return cached;
    if (!this.vrm) return null;
    const loader = new GLTFLoader();
    loader.register((parser) => new VRMAnimationLoaderPlugin(parser));
    const gltf = await loader.loadAsync(url);
    const anims = gltf.userData.vrmAnimations as VRMAnimation[] | undefined;
    if (!anims || anims.length === 0) {
      console.warn(`[pet] ${url} contains no VRM animation`);
      return null;
    }
    const clip = createVRMAnimationClip(anims[0], this.vrm);
    this.clipCache.set(url, clip);
    return clip;
  }

  /** Play a `.vrma` clip on the body, crossfading from whatever's playing.
   *  Loops hold (idle / walk); one-shots clamp then hand the body back to the
   *  procedural layer. While a clip plays it owns the managed bones — the face
   *  (emotion, blink, lip-sync) keeps riding on top. */
  async playClip(url: string, opts: { loop?: boolean } = {}): Promise<void> {
    if (!this.mixer) {
      console.warn('[pet] playClip before the model is ready — ignored');
      return;
    }
    let clip: THREE.AnimationClip | null = null;
    try {
      clip = await this.loadClip(url);
    } catch (e) {
      console.warn(`[pet] clip load failed: ${url}`, e);
    }
    if (!clip || !this.mixer) return;

    const next = this.mixer.clipAction(clip);
    next.reset();
    next.loop = opts.loop ? THREE.LoopRepeat : THREE.LoopOnce;
    next.clampWhenFinished = !opts.loop;
    next.play();
    if (this.currentAction && this.currentAction !== next) {
      next.crossFadeFrom(this.currentAction, 0.3, false);
    }
    this.currentAction = next;
  }

  setCursor(x: number, y: number) {
    this.cursor.x = clamp(x, -1, 1);
    this.cursor.y = clamp(y, -1, 1);
  }

  // --- Per-frame drivers ---

  private updateBlink(dt: number) {
    if (this.blinkProgress >= 0) {
      this.blinkProgress += dt / 0.16;
      this.blinkValue = this.blinkProgress < 0.5
        ? this.blinkProgress * 2
        : Math.max(0, (1 - this.blinkProgress) * 2);
      if (this.blinkProgress >= 1) { this.blinkProgress = -1; this.blinkValue = 0; }
      return;
    }
    this.blinkTimer -= dt;
    if (this.blinkTimer <= 0) {
      this.blinkProgress = 0;
      this.blinkTimer = 2.5 + Math.random() * 3.5;
    }
  }

  private updateLipSync(dt: number) {
    // Damp toward the externally-set target (the audio envelope while she speaks,
    // 0 when silent) — actual amplitude sync, smoothed.
    this.mouthValue = THREE.MathUtils.damp(this.mouthValue, this.mouthTarget, 18, dt);
  }

  private applyExpressions() {
    const em = this.vrm?.expressionManager;
    if (!em) return;
    for (const e of EMOTIONS) {
      em.setValue(e, e === this.emotion ? this.emotionWeight : 0);
    }
    em.setValue('blink', this.blinkValue);
    em.setValue('aa', this.mouthValue);
  }

  private computeOffsets(t: number, breath: number): Map<VRMHumanBoneName, Offset> {
    const off = new Map<VRMHumanBoneName, Offset>();
    const add = (b: VRMHumanBoneName, x = 0, y = 0, z = 0) => {
      const cur = off.get(b) ?? { x: 0, y: 0, z: 0 };
      off.set(b, { x: cur.x + x, y: cur.y + y, z: cur.z + z });
    };

    // Relax arms from the VRM T-pose into a natural A-pose.
    add('leftUpperArm', 0, 0, 1.2);
    add('rightUpperArm', 0, 0, -1.2);
    add('leftLowerArm', 0, 0, 0.15);
    add('rightLowerArm', 0, 0, -0.15);

    // Breathing.
    add('upperChest', breath * 0.05);
    add('spine', breath * 0.02);

    // Gaze head-follow (eyes via lookAt).
    add('head', -this.cursor.y * 0.12, this.cursor.x * 0.22);
    add('neck', this.cursor.x * 0.06);

    // One-shot gesture overlay (Express actions). x=pitch, y=yaw, z=arm raise.
    const a = this.action;
    if (a) {
      const p = a.t / a.dur; // 0..1 progress
      const env = Math.sin(p * Math.PI); // smooth rise→fall, 0 at the ends
      switch (a.name) {
        case 'nod':
          add('head', Math.sin(p * Math.PI * 4) * 0.3);
          break;
        case 'shake':
          add('head', 0, Math.sin(p * Math.PI * 4) * 0.4);
          break;
        case 'wave':
          add('rightUpperArm', 0, 0, env * 1.7);
          add('rightLowerArm', 0, 0, env * 0.3 + Math.sin(p * Math.PI * 8) * 0.5 * env);
          add('head', 0, env * 0.08);
          break;
        case 'bow':
          add('spine', env * 0.45);
          add('neck', env * 0.15);
          add('head', env * 0.18);
          break;
        case 'dance': {
          const ph = t * 6;
          add('spine', 0, Math.sin(ph) * 0.12, Math.sin(ph * 0.5) * 0.1);
          add('upperChest', 0, 0, Math.sin(ph) * 0.06);
          add('head', 0, Math.sin(ph + 0.5) * 0.12);
          add('leftUpperArm', 0, 0, Math.sin(ph) * 0.4);
          add('rightUpperArm', 0, 0, -Math.sin(ph) * 0.4);
          break;
        }
        case 'cheer':
          add('leftUpperArm', 0, 0, env * -2.6);
          add('rightUpperArm', 0, 0, env * 2.6);
          add('head', env * -0.12);
          break;
      }
    }

    return off;
  }

  private applyPose(off: Map<VRMHumanBoneName, Offset>) {
    const hum = this.vrm?.humanoid;
    if (!hum) return;
    for (const name of MANAGED) {
      const node = hum.getNormalizedBoneNode(name);
      const rest = this.rest.get(name);
      if (!node || !rest) continue;
      node.quaternion.copy(rest);
      const o = off.get(name);
      if (o) { node.rotateZ(o.z); node.rotateX(o.x); node.rotateY(o.y); }
    }
  }

  private updateTransform(t: number) {
    if (!this.vrm) return;
    const s = this.vrm.scene;
    let y = this.baseY + Math.sin(t * 1.4) * 0.004; // idle bob
    const a = this.action;
    if (a?.name === 'dance') y += Math.abs(Math.sin(a.t * 6)) * 0.02;
    else if (a?.name === 'cheer') y += Math.sin((a.t / a.dur) * Math.PI) * 0.1; // hop
    s.position.set(0, y, 0);
  }

  /** appear / disappear: scale + fade the whole avatar toward `visibility`.
   *  Skips per-frame material churn once fully settled at visible. */
  private applyVisibility() {
    if (!this.vrm) return;
    if (Math.abs(this.visibility - this.visApplied) < 0.004 && this.visibility > 0.999) return;
    this.visApplied = this.visibility;
    const v = this.visibility;
    const s = this.vrm.scene;
    s.scale.setScalar(Math.max(0.001, v));
    s.visible = v > 0.01;
    s.traverse((o) => {
      const mats = (o as THREE.Mesh).material;
      const list = Array.isArray(mats) ? mats : mats ? [mats] : [];
      for (const m of list) {
        m.transparent = true;
        m.opacity = v;
        m.depthWrite = v > 0.5;
      }
    });
  }

  private animate = () => {
    this.raf = requestAnimationFrame(this.animate);
    const dt = Math.min(this.clock.getDelta(), 0.05);
    const t = this.clock.elapsedTime;

    this.emotionWeight = THREE.MathUtils.damp(
      this.emotionWeight, this.emotion === 'neutral' ? 0 : 1, 8, dt,
    );

    if (this.vrm) {
      // A clip, while playing, drives the body. Advance it first so the face
      // layer (blink / lip-sync / emotion) can ride on top before vrm.update.
      const clipActive = this.currentAction !== null;
      if (this.mixer) this.mixer.update(dt);

      this.lookTarget.position.set(
        this.cursor.x * 0.6,
        this.gazeY + this.cursor.y * 0.4,
        this.camera.position.z,
      );

      if (!clipActive) {
        // Procedural body: idle posture + one-shot gesture overlay.
        if (this.action) {
          this.action.t += dt;
          if (this.action.t >= this.action.dur) this.action = null;
        }
        const breath = Math.sin(t * 1.4);
        this.applyPose(this.computeOffsets(t, breath));
        this.updateTransform(t);
      }

      this.updateBlink(dt);
      this.updateLipSync(dt);
      this.applyExpressions();
      this.visibility = THREE.MathUtils.damp(this.visibility, this.visible ? 1 : 0, 9, dt);
      this.applyVisibility();
      this.vrm.update(dt);
    }

    this.renderer.render(this.scene, this.camera);
  };

  private frameCamera(vrm: VRM) {
    const box = new THREE.Box3().setFromObject(vrm.scene);
    const center = box.getCenter(new THREE.Vector3());
    const size = box.getSize(new THREE.Vector3());
    this.baseY = vrm.scene.position.y;
    this.gazeY = center.y + size.y * 0.38;

    const fov = (this.camera.fov * Math.PI) / 180;
    const fitH = size.y / 2 / Math.tan(fov / 2);
    const fitW = size.x / 2 / (Math.tan(fov / 2) * this.camera.aspect);
    const dist = Math.max(fitH, fitW) * 1.12;

    this.camera.position.set(center.x, center.y, center.z + dist);
    this.camera.lookAt(center.x, center.y, center.z);
  }

  private resize() {
    const w = this.canvas.clientWidth || 1;
    const h = this.canvas.clientHeight || 1;
    this.camera.aspect = w / h;
    this.camera.updateProjectionMatrix();
    this.renderer.setSize(w, h, false);
    if (this.vrm) this.frameCamera(this.vrm);
  }

  dispose() {
    cancelAnimationFrame(this.raf);
    this.resizeObs.disconnect();
    this.mixer?.stopAllAction();
    if (this.vrm) VRMUtils.deepDispose?.(this.vrm.scene);
    this.renderer.dispose();
  }
}
