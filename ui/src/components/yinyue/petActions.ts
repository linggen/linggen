/**
 * Pet action manifest — the web side of `public/anim/actions.json`.
 *
 * One source of truth, two readers: the Rust engine `include_str!`s the same
 * file to build the `Express` tool vocabulary (it reads `name` + `use_when`);
 * here we fetch it to dispatch each intent to the renderer by its `render`
 * kind. Adding an intent is one entry in the manifest — no code change here.
 *
 * `render`:
 *   - `proc`       → a procedural bone gesture (PetStage.playAction)
 *   - `clip`       → a `.vrma` clip via AnimationMixer (Phase 2; `clips` empty
 *                    today → the intent just carries its `emotion` until a clip
 *                    is dropped in)
 *   - `visibility` → show / hide the whole avatar (scale + opacity)
 */
import { _originalFetch } from '../../lib/fetchProxy';

export type RenderKind = 'proc' | 'clip' | 'visibility';

export interface PetIntent {
  name: string;
  use_when: string;
  category?: string;
  type?: 'once' | 'loop' | 'pose';
  render: RenderKind;
  /** Sustained face mood applied when the model didn't set one explicitly. */
  emotion?: string | null;
  /** render === 'proc' — the PetStage gesture name. */
  proc?: string;
  /** render === 'clip' — candidate `.vrma` URLs; engine picks one at random. */
  clips?: string[];
  /** render === 'visibility' — true = appear, false = disappear. */
  visible?: boolean;
}

interface Manifest {
  intents: PetIntent[];
}

// Fetched once and memoized — the manifest is static for the session.
let cache: Promise<Map<string, PetIntent>> | null = null;

export function loadIntents(): Promise<Map<string, PetIntent>> {
  if (!cache) {
    cache = _originalFetch('/anim/actions.json')
      .then((r) => {
        if (!r.ok) throw new Error(`actions.json ${r.status}`);
        return r.json() as Promise<Manifest>;
      })
      .then((m) => new Map(m.intents.map((i) => [i.name, i])))
      .catch((e) => {
        console.warn('[pet] action manifest load failed', e);
        return new Map<string, PetIntent>();
      });
  }
  return cache;
}

/** Pick a clip variant at random — variant choice carries no semantic signal,
 *  so it stays out of the LLM's hands. Returns null until clips are added. */
export function pickClip(intent: PetIntent): string | null {
  const clips = intent.clips;
  if (!clips || clips.length === 0) return null;
  return clips[Math.floor(Math.random() * clips.length)];
}
