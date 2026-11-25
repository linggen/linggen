// The engine's shared hold-to-talk control (linggen/shared/hold-to-talk.js:
// the press → record → hear → send control, the WAV encoder and the
// microphone's recorder, one file), bundled through the `@linggen/hold-to-talk`
// alias in vite.config.ts. The file is plain JavaScript; these are its shapes
// for the UI's compiler. Keep them beside it: a change to one is a change to both.
declare module '@linggen/hold-to-talk' {
  export const HOLD_MS: number;
  export const DRAG_PX: number;
  export const NOTE_MS: number;
  export const HEARD_MS: number;
  export const HEARS_MS: number;
  export const HEAR_BUDGET_MS: number;
  export const SPEECH_RATE: number;
  export const MAX_SPEECH_SECONDS: number;
  export const WORDS: {
    readonly listening: string;
    readonly thinking: string;
    readonly nothing: string;
    readonly cannotHear: string;
    readonly micBlocked: string;
    readonly micAllowed: string;
    readonly noMic: string;
    readonly failed: string;
  };

  export type TalkPhase =
    | 'idle'
    | 'pressed' // down, not yet a hold
    | 'opening' // a hold; the microphone is being asked for
    | 'listening' // the microphone is open
    | 'refused' // a hold that cannot record; waits for the release
    | 'hearing' // let go; the clip is being heard, then sent
    | 'replying'; // sent; her answer is on its way

  /** What the surface draws: the phase, the words heard, one quiet line. */
  export interface TalkView {
    phase: TalkPhase;
    heard: string | null;
    note: string | null;
  }

  export function talkLabel(view: TalkView, petThinking: boolean): string | null;
  export function isTalkKey(e: { code: string; repeat: boolean; metaKey: boolean; ctrlKey: boolean; altKey: boolean; target: unknown }): boolean;

  interface TrackLike { stop(): void }
  interface StreamLike { getTracks(): TrackLike[] }
  interface NodeLike { connect(to: unknown): unknown; disconnect(): void }
  /** The recorder inside the audio context: it posts the microphone's samples in blocks. */
  interface TapLike extends NodeLike {
    port: { onmessage: ((e: { data: Float32Array }) => void) | null };
  }
  export interface AudioContextLike {
    sampleRate: number;
    destination: unknown;
    resume(): Promise<void>;
    suspend(): Promise<void>;
    /** The recorder is loaded into the context (once). */
    ready(): Promise<void>;
    createMediaStreamSource(stream: StreamLike): NodeLike;
    createTap(): TapLike;
  }
  interface Listenable {
    addEventListener(type: string, fn: () => void): void;
    removeEventListener(type: string, fn: () => void): void;
  }

  /** What the page's door did with a line: took it, or refused — with its own
   *  one line of reason when it gave one. */
  export type LineSent = { sent: true } | { sent: false; reason: string | null };

  export interface TalkDeps {
    /** `navigator.mediaDevices`, or undefined where the page has none. */
    mediaDevices: () => { getUserMedia(c: unknown): Promise<StreamLike> } | undefined;
    /** Made inside the press, where the page allows a sound context to run. */
    newAudioContext: () => AudioContextLike;
    /** Whether holding can ever be granted a microphone here; when not, a press stays a tap. */
    offered: () => boolean;
    /** The page's own `fetch` (data channel); read at call time. */
    fetch: (url: string, init: { method: string; headers: Record<string, string>; body?: string }) => Promise<{ ok: boolean; status: number; json(): Promise<unknown> }>;
    /** The line as a typed one would go. */
    sendLine: (text: string) => Promise<LineSent>;
    /** The one stop the page's mute uses. */
    cutOff: () => void;
    now: () => number;
    setTimer: (fn: () => void, ms: number) => unknown;
    clearTimer: (id: unknown) => void;
    win: Listenable;
    doc: Listenable & { hidden: boolean };
    /** The language the clip is in (a code, or 'auto' = tell the model nothing); without it the engine decides. */
    language?: string;
    /** Default true. False: a hold that finds her unable to hear sends no knock (no download starts). */
    knock?: boolean;
  }

  /** What the engine says of her hearing (asked, never guessed). `next` is the engine's word for what would help. */
  export interface HearingStatus {
    hears: boolean;
    next: 'never' | 'download' | 'working' | 'downloading' | 'retry' | null;
    reason: string | null;
    failed?: true;
  }

  type Source = 'pointer' | 'key';

  export interface HoldToTalk {
    view(): TalkView;
    subscribe(fn: () => void): () => void;
    /** Pointer down or key down. */
    press(source: Source): void;
    /** Pointer up or key up: 'tap' if it never became a hold (let the avatar's
     *  click proceed), 'talk' if it did (swallow that click). */
    release(source: Source): 'tap' | 'talk';
    /** Drop a press that has not been heard yet: microphone off, nothing sent. */
    cancel(): void;
    /** Whether a press here can become a talk at all. */
    offered(): boolean;
    /** Ask whether she can hear. Read only: starts nothing. */
    status(): Promise<HearingStatus>;
    /** A person's yes: begin the fetch or load hearing waits on; answers how she then stands. */
    start(): Promise<HearingStatus>;
  }

  export function createHoldToTalk(deps: TalkDeps): HoldToTalk;
  /** A fresh audio context with the recorder ready to load. */
  export function newSpeechContext(): AudioContextLike;
  /** 16 kHz mono samples (-1..1) as one WAV file. */
  export function encodeWav(samples: Float32Array): Uint8Array;
  /** Mono samples at `fromRate` as mono samples at 16 kHz, cut at 60 s. */
  export function toSpeechSamples(input: Float32Array, fromRate: number): Float32Array;
  /** Base64 of bytes, in slices. */
  export function toBase64(bytes: Uint8Array): string;
}
