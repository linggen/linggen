// The engine's shared sky painter (linggen/shared/sky.js), bundled through the
// `@linggen/sky` alias in vite.config.ts.
declare module '@linggen/sky' {
  export interface SkyHandle {
    canvas: HTMLCanvasElement;
    readonly mode: 'motion' | 'still';
    set(sky: { kind: string; is_day?: boolean }): void;
    destroy(): void;
  }
  export interface SkyOptions {
    kind?: string;
    is_day?: boolean;
    dark?: boolean;
    lightning?: boolean;
    density?: number;
    still?: boolean;
    auto?: boolean;
    cover?: () => DOMRect[];
    overlay?: boolean;
    onMode?: (mode: 'motion' | 'still') => void;
  }
  export const KINDS: string[];
  export function mountSky(host: HTMLElement, opts?: SkyOptions): SkyHandle;
}
