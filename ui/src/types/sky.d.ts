// The engine's shared sky painter (linggen/shared/sky.js), bundled through the
// `@linggen/sky` alias in vite.config.ts.
declare module '@linggen/sky' {
  export interface SkyHandle {
    canvas: HTMLCanvasElement;
    readonly mode: 'motion' | 'still';
    set(sky: SkyState): void;
    destroy(): void;
  }
  export interface SkyWind { force: number; dir?: number }
  export interface SkyState { kind: string; is_day?: boolean; wind?: SkyWind | null; season?: string | null; festival?: string | null }
  export interface SkyOptions {
    kind?: string;
    is_day?: boolean;
    wind?: SkyWind | null;
    season?: string | null;
    festival?: string | null;
    dark?: boolean;
    lightning?: boolean;
    density?: number;
    still?: boolean;
    auto?: boolean;
    cover?: () => DOMRect[];
    overlay?: boolean;
    /** The moment the moon's phase is read at (tests, pictures); none: now. */
    now?: number | Date;
    onMode?: (mode: 'motion' | 'still') => void;
  }
  export const KINDS: string[];
  /** The moon's mean phase: `lit` 0 (new) … 1 (full), `waxing` (lit on the right in the northern sky). */
  export function moonPhase(when?: number | Date): { lit: number; waxing: boolean };
  export function mountSky(host: HTMLElement, opts?: SkyOptions): SkyHandle;
}
