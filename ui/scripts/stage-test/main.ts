// A minimal page that mounts the app's real PetStage with a frozen, hand-stepped clock — so two
// builds of PetStage can be photographed at the same moment of the same clip. Not shipped.
import { PetStage } from '../../src/components/yinyue/PetStage';

// Virtual time: THREE.Clock reads performance.now(); requestAnimationFrame is ours to fire.
let nowMs = 1000;
performance.now = () => nowMs;
let pending: FrameRequestCallback | null = null;
window.requestAnimationFrame = (cb) => { pending = cb; return 1; };
window.cancelAnimationFrame = () => { pending = null; };
let seed = 12345;
Math.random = () => { seed = (Math.imul(seed, 1664525) + 1013904223) >>> 0; return seed / 2 ** 32; };

let stage: PetStage | null = null;

(window as unknown as { stageTest: unknown }).stageTest = {
  async setup() {
    stage = new PetStage(document.getElementById('c') as HTMLCanvasElement);
    await stage.load('/yinyue.vrm');
  },
  step(n: number, dt = 1 / 30) {
    for (let i = 0; i < n; i++) {
      nowMs += dt * 1000;
      const cb = pending;
      pending = null;
      cb?.(nowMs);
    }
  },
  async play(url: string, loop: boolean) {
    return stage ? stage.playClip(url, { loop }) : -1;
  },
};
