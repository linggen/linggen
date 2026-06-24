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
import { loadMixamoAnimation, trimClip } from './petMixamo';

export type EmotionName = 'neutral' | 'happy' | 'angry' | 'sad' | 'relaxed';
export type ActionName = 'nod' | 'shake' | 'wave' | 'bow' | 'dance' | 'cheer';

/** One resolved step of a sequence — the renderer-facing shape (no manifest
 *  knowledge; YinyueAvatar resolves intent names to these). */
export interface SeqStep {
  render: 'proc' | 'clip' | 'visibility';
  proc?: ActionName;
  clipUrl?: string;
  loop?: boolean;
  visible?: boolean;
}

const EMOTIONS: Exclude<EmotionName, 'neutral'>[] = ['happy', 'angry', 'sad', 'relaxed'];

// One-shot gesture durations (seconds).
const ACTION_DUR: Record<ActionName, number> = {
  nod: 0.9, shake: 0.9, wave: 1.6, bow: 1.8, dance: 3.0, cheer: 1.6,
};

// A set mood is a transient reaction — it fades back to neutral after this long,
// so her eyes (which the happy/sad blendshapes narrow) return to open + blinking.
const MOOD_DECAY_S = 6;

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
  private emotionSetAt = 0; // clock time the current mood was set (for decay)
  private blinkValue = 0;
  private blinkTimer = 2;
  private blinkProgress = -1;
  private mouthValue = 0;
  private mouthTarget = 0; // 0..1, set externally from the audio envelope
  private action: ActiveAction | null = null; // one-shot gesture overlay
  private visible = true; // sustained show/hide target (appear / disappear)
  private visibility = 1; // animated 0..1, damps toward visible ? 1 : 0
  private visApplied = 1; // last opacity/scale written (skip churn once settled)
  private seqToken = 0; // bumped per playSequence; a newer call supersedes an older
  private resetPosePending = false; // a clip ended → reset bones it touched that we don't manage
  private thinking = false; // a reply is in flight → hold a pondering pose
  private thinkWeight = 0; // eased 0..1 so the procedural pondering pose blends in/out
  // Optional Mixamo "Thinking" clip (real hand-on-chin); falls back to the
  // procedural head-cock if absent. undefined = not yet loaded, null = none.
  // Trimmed to the raise→hold window (hand reaches the chin ~t1.0 and holds to
  // ~t2.75; it lowers after t3.0). Ending at 2.5 + clampWhenFinished keeps the
  // hand at the chin for the whole wait instead of dropping it.
  private thinkingClipUrl = '/anim/thinking.fbx#0,2.5';
  private thinkingClip: THREE.AnimationClip | null | undefined = undefined;
  private thinkingAction: THREE.AnimationAction | null = null;
  // Resting idle — a LOOPING mocap clip is her default pose (not a procedural
  // A-pose). idleClipUrls[0] is the default she returns to; the engine
  // crossfades to a different pool clip every so often for variety, so she's
  // never a frozen statue. Procedural posing is only a pre-load fallback.
  private idleClipUrls: string[] = ['/anim/idle2.fbx', '/anim/idle.fbx'];
  private idleAction: THREE.AnimationAction | null = null;
  private idleActionUrl: string | null = null; // which pool clip is currently looping
  private idleStarting = false;
  private idleSwapAt = 0; // clock time of the next variety swap
  // Talking body loop — plays while her voice is playing (gesture overrides it).
  private speaking = false;
  private talkingClipUrl: string | null = '/anim/talking.fbx';
  private talkingClip: THREE.AnimationClip | null | undefined = undefined;
  private talkingAction: THREE.AnimationAction | null = null;
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
    this.mixer.addEventListener('finished', (e) => {
      const action = (e as unknown as { action?: THREE.AnimationAction }).action;
      // Thinking clamps + holds its final pose (hand at chin) — leave it running.
      if (action && action === this.thinkingAction) return;
      // A one-shot clip ended → stop it (so the mixer stops holding its final
      // frame) and hand the body back to the procedural layer, flagging a reset
      // for the bones the clip moved that we don't manage (hands, legs, hips).
      action?.stop();
      this.currentAction = null;
      this.resetPosePending = true;
    });
  }

  // --- Director API ---

  setEmotion(name: EmotionName) {
    this.emotion = name;
    this.emotionSetAt = this.clock.elapsedTime;
  }

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

  /** Toggle the pondering state while a reply is in flight. Plays the Mixamo
   *  "Thinking" clip (real hand-on-chin) if available, else the procedural
   *  head-cock pose carries it. */
  setThinking(v: boolean) {
    if (v === this.thinking) return;
    this.thinking = v;
    if (v) void this.startThinkingClip();
    else this.stopThinkingClip();
  }

  /** Toggle the talking body loop (driven on while her voice plays). The base
   *  layer (ensureBase) swaps idle↔talking; here we just interrupt the *other*
   *  base clip so the switch is prompt. A gesture/thinking still overrides. */
  setSpeaking(v: boolean) {
    if (v === this.speaking) return;
    this.speaking = v;
    if (v) {
      if (this.currentAction && this.currentAction === this.idleAction) {
        this.idleAction.stop();
        this.currentAction = null;
        this.resetPosePending = true;
      }
    } else if (this.currentAction && this.currentAction === this.talkingAction) {
      this.talkingAction!.stop();
      this.currentAction = null;
      this.resetPosePending = true;
    }
  }

  private async startThinkingClip() {
    if (!this.mixer || !this.vrm) return;
    if (this.thinkingClip === undefined) {
      // Lazy-load + retarget the FBX once; null if missing/unparseable.
      this.thinkingClip = await loadMixamoAnimation(this.thinkingClipUrl, this.vrm).catch((e) => {
        console.warn('[pet] thinking clip load failed — using procedural pose', e);
        return null;
      });
    }
    // Thinking may have ended while loading, or there's no clip → procedural carries it.
    if (!this.thinking || !this.thinkingClip || !this.mixer) return;
    const action = this.mixer.clipAction(this.thinkingClip);
    action.reset();
    action.loop = THREE.LoopOnce;
    action.clampWhenFinished = true; // raise the hand to the chin and HOLD it while thinking
    action.play();
    if (this.currentAction && this.currentAction !== action) {
      action.crossFadeFrom(this.currentAction, 0.3, false);
    }
    this.currentAction = action;
    this.thinkingAction = action;
  }

  private stopThinkingClip() {
    if (!this.thinkingAction) return;
    this.thinkingAction.stop();
    if (this.currentAction === this.thinkingAction) {
      this.currentAction = null;
      this.resetPosePending = true; // hand the body back (idle clip / procedural)
    }
    this.thinkingAction = null;
  }

  /** The BASE layer whenever no gesture/thinking owns the body: the talking
   *  clip while her voice plays, otherwise a LOOPING idle clip as her default
   *  resting pose — crossfading to a different pool clip every so often for
   *  variety. The procedural A-pose only shows before the first clip loads (or
   *  if every idle clip fails to retarget). */
  private async ensureBase() {
    if (!this.mixer || !this.vrm || this.idleStarting) return;
    if (this.thinking) return; // thinking owns the body
    // A non-idle clip (gesture / talking) owns the body → leave it be.
    if (this.currentAction && this.currentAction !== this.idleAction) return;

    // Speaking → loop the talking body clip for the whole utterance.
    if (this.speaking && this.talkingClipUrl) {
      if (this.talkingClip === undefined) {
        this.idleStarting = true;
        this.talkingClip = await this.loadClip(this.talkingClipUrl).catch(() => null);
        this.idleStarting = false;
      }
      if (!this.speaking || !this.talkingClip || !this.mixer) return;
      const fromIdle = this.currentAction === this.idleAction ? this.idleAction : null;
      const a = this.mixer.clipAction(this.talkingClip);
      a.reset();
      a.loop = THREE.LoopRepeat;
      a.play();
      if (fromIdle && fromIdle !== a) a.crossFadeFrom(fromIdle, 0.3, true);
      this.talkingAction = a;
      this.currentAction = a;
      this.resetPosePending = false;
      return;
    }

    // Otherwise → a looping idle clip is her default. (Re)start on the default
    // clip when nothing's playing; once it's looped a while, crossfade to a
    // different pool clip so she varies.
    if (this.idleClipUrls.length === 0) return;
    const idleActive = !!this.idleAction && this.currentAction === this.idleAction;
    if (idleActive && this.clock.elapsedTime < this.idleSwapAt) return; // current idle still has time
    const url = idleActive ? this.pickIdleUrl() : this.idleClipUrls[0]; // default on entry, variety on swap
    this.idleStarting = true;
    const clip = await this.loadClip(url).catch(() => null);
    this.idleStarting = false;
    if (!clip || this.thinking || this.speaking || !this.mixer) return;
    if (this.currentAction && this.currentAction !== this.idleAction) return; // a gesture slipped in while loading
    const fromIdle = this.idleAction && this.currentAction === this.idleAction ? this.idleAction : null;
    const a = this.mixer.clipAction(clip);
    a.reset();
    a.loop = THREE.LoopRepeat;
    a.play();
    if (fromIdle && fromIdle !== a) a.crossFadeFrom(fromIdle, 0.5, true);
    this.idleAction = a;
    this.idleActionUrl = url;
    this.currentAction = a;
    this.resetPosePending = false;
    this.idleSwapAt = this.clock.elapsedTime + 12 + Math.random() * 10; // next variety swap in 12–22s
  }

  /** Pick a pool clip for an idle variety swap — preferring one different from
   *  the clip currently looping so the change is visible. */
  private pickIdleUrl(): string {
    const choices = this.idleClipUrls.filter((u) => u !== this.idleActionUrl);
    const from = choices.length > 0 ? choices : this.idleClipUrls;
    return from[Math.floor(Math.random() * from.length)];
  }

  /** Load + cache a `.vrma`, retargeted to this VRM's humanoid rig. */
  private async loadClip(url: string): Promise<THREE.AnimationClip | null> {
    const cached = this.clipCache.get(url);
    if (cached) return cached;
    if (!this.vrm) return null;
    // Optional `#start,end` (seconds) trim window appended to the URL.
    const [src, frag] = url.split('#');
    let clip: THREE.AnimationClip | null = null;
    if (src.endsWith('.fbx')) {
      // Mixamo FBX → retargeted to this VRM rig at runtime (no Blender).
      clip = await loadMixamoAnimation(src, this.vrm);
    } else {
      // VRMA — native VRM humanoid animation.
      const loader = new GLTFLoader();
      loader.register((parser) => new VRMAnimationLoaderPlugin(parser));
      const gltf = await loader.loadAsync(src);
      const anims = gltf.userData.vrmAnimations as VRMAnimation[] | undefined;
      if (!anims || anims.length === 0) {
        console.warn(`[pet] ${src} contains no VRM animation`);
        return null;
      }
      clip = createVRMAnimationClip(anims[0], this.vrm);
    }
    if (clip && frag) clip = trimClip(clip, frag);
    if (clip) {
      console.info(`[pet] clip ${url} dur=${clip.duration.toFixed(2)}s`); // helps tune trim windows
      this.clipCache.set(url, clip);
    }
    return clip;
  }

  /** Play a `.vrma` clip on the body, crossfading from whatever's playing.
   *  Loops hold (idle / walk); one-shots clamp then hand the body back to the
   *  procedural layer. While a clip plays it owns the managed bones — the face
   *  (emotion, blink, lip-sync) keeps riding on top. */
  async playClip(url: string, opts: { loop?: boolean } = {}): Promise<number> {
    if (!this.mixer) {
      console.warn('[pet] playClip before the model is ready — ignored');
      return 0;
    }
    let clip: THREE.AnimationClip | null = null;
    try {
      clip = await this.loadClip(url);
    } catch (e) {
      console.warn(`[pet] clip load failed: ${url}`, e);
    }
    if (!clip || !this.mixer) return 0;

    const next = this.mixer.clipAction(clip);
    next.reset();
    next.loop = opts.loop ? THREE.LoopRepeat : THREE.LoopOnce;
    next.clampWhenFinished = !opts.loop;
    next.play();
    if (this.currentAction && this.currentAction !== next) {
      next.crossFadeFrom(this.currentAction, 0.3, false);
    }
    this.currentAction = next;
    return clip.duration; // seconds
  }

  /** Stop any playing clip and hand the body back to the procedural layer. */
  private stopClip() {
    if (this.currentAction) {
      this.mixer?.stopAllAction();
      this.currentAction = null;
      this.resetPosePending = true;
    }
  }

  /** Play resolved steps in order, each dwelling for its natural duration.
   *  A newer call supersedes a running one (token check after every await). */
  async playSequence(steps: SeqStep[]): Promise<void> {
    const token = ++this.seqToken;
    for (const step of steps) {
      if (token !== this.seqToken) return; // superseded by a newer Express
      const dwellMs = await this.playStep(step);
      if (token !== this.seqToken) return;
      if (steps.length > 1) {
        await new Promise((r) => setTimeout(r, dwellMs + 120)); // brief beat between
      }
    }
  }

  /** Dispatch one step by render kind; returns how long to dwell before the next. */
  private async playStep(step: SeqStep): Promise<number> {
    switch (step.render) {
      case 'proc':
        if (!step.proc) return 0;
        this.stopClip();
        this.playAction(step.proc);
        return ACTION_DUR[step.proc] * 1000;
      case 'visibility':
        this.stopClip();
        this.setVisible(step.visible ?? true);
        return 600;
      case 'clip': {
        if (!step.clipUrl) return 0;
        const dur = await this.playClip(step.clipUrl, { loop: step.loop });
        return step.loop ? 2000 : Math.max(300, dur * 1000);
      }
      default:
        return 0;
    }
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

    // Relax arms from the normalized T-pose down into a natural A-pose. On our
    // VRM1 rig the upper-arm roll is mirrored (+Z raises the left, -Z the
    // right), so lowering to an A-pose takes the opposite sign on each side.
    add('leftUpperArm', 0, 0, -1.2);
    add('rightUpperArm', 0, 0, 1.2);
    add('leftLowerArm', 0, 0, -0.15);
    add('rightLowerArm', 0, 0, 0.15);

    // Breathing.
    add('upperChest', breath * 0.05);
    add('spine', breath * 0.02);

    // Ambient idle — a slow weight shift + sway so she stays gently alive
    // when standing, never a frozen statue (layered under breath + any gesture).
    add('spine', 0, Math.sin(t * 0.45) * 0.03, Math.sin(t * 0.3) * 0.02);
    add('upperChest', 0, 0, Math.sin(t * 0.45 + 0.6) * 0.02);
    add('head', Math.sin(t * 0.6) * 0.015, Math.sin(t * 0.33) * 0.04);
    add('neck', 0, Math.sin(t * 0.33) * 0.02);

    // Thinking — a clear pondering pose while a reply is in flight: head tilted
    // and dipped with a slow "hmm" nod, a slight lean, and a hand brought up
    // toward the chin (best-effort, no hand bones). All eased by thinkWeight.
    const tw = this.thinkWeight;
    if (tw > 0.01) {
      const hmm = Math.sin(t * 1.6);
      // A cocked head (sideways tilt) with a slow mulling bob — reads as
      // pondering WITHOUT craning her backward: tiny dip, no back-lean, gaze
      // kept roughly level (see the gaze block). z = roll (the head-tilt).
      add('head', 0.04 * tw, 0.05 * tw, (0.16 + hmm * 0.05) * tw);
      add('neck', 0.02 * tw, 0.03 * tw, 0.05 * tw);
    }

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
          add('rightUpperArm', 0, 0, env * -1.7); // -Z raises the right arm
          add('rightLowerArm', 0, 0, env * -0.3 + Math.sin(p * Math.PI * 8) * 0.5 * env);
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
          add('leftUpperArm', 0, 0, -Math.sin(ph) * 0.4);
          add('rightUpperArm', 0, 0, Math.sin(ph) * 0.4);
          break;
        }
        case 'cheer':
          add('leftUpperArm', 0, 0, env * 2.6); // raise both arms overhead
          add('rightUpperArm', 0, 0, env * -2.6);
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

    // Mood is transient — fade it back to neutral so her eyes reopen.
    if (this.emotion !== 'neutral' && this.clock.elapsedTime - this.emotionSetAt > MOOD_DECAY_S) {
      this.emotion = 'neutral';
    }
    this.emotionWeight = THREE.MathUtils.damp(
      this.emotionWeight, this.emotion === 'neutral' ? 0 : 1, 8, dt,
    );

    if (this.vrm) {
      // Base layer: talking clip while speaking, else idle when nothing else plays.
      void this.ensureBase();
      // A clip, while playing, drives the body. Advance it first so the face
      // layer (blink / lip-sync / emotion) can ride on top before vrm.update.
      const clipActive = this.currentAction !== null;
      if (this.mixer) this.mixer.update(dt);
      this.thinkWeight = THREE.MathUtils.damp(this.thinkWeight, this.thinking ? 1 : 0, 6, dt);

      // Gaze: cursor-follow with a slow autonomous drift (never a fixed stare);
      // while thinking, blend toward a wandering up-and-away look.
      const driftX = Math.sin(t * 0.23) * 0.22 + Math.sin(t * 0.071) * 0.12;
      const driftY = Math.sin(t * 0.17) * 0.12;
      let gx = (this.cursor.x + driftX) * 0.6;
      let gy = this.gazeY + (this.cursor.y + driftY) * 0.4;
      if (this.thinkWeight > 0.01) {
        const w = this.thinkWeight;
        const tx = 0.28 + Math.sin(t * 0.5) * 0.18; // glance aside, gently wandering
        const ty = this.gazeY + 0.05 + Math.sin(t * 0.8) * 0.04; // roughly level — not craned up
        gx = gx * (1 - w) + tx * w;
        gy = gy * (1 - w) + ty * w;
      }
      this.lookTarget.position.set(gx, gy, this.camera.position.z);

      if (!clipActive) {
        // A clip just ended → clear its residue on bones we don't manage
        // (hands/legs/hips) before the procedural layer takes back over.
        if (this.resetPosePending) {
          this.vrm.humanoid?.resetNormalizedPose();
          this.resetPosePending = false;
        }
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
