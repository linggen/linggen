/**
 * The page's audio context for hearing (lib/holdToTalk.mts hands it in through
 * yinyueTalk.ts): an AudioWorklet taps the microphone's samples in blocks and
 * posts them to the page, so capture never runs on the thread that draws her.
 * The worklet's code is a string made into a Blob URL — no second file to
 * serve, and nothing fetched over the network.
 *
 * Why a worklet and not MediaRecorder: the recorder yields a compressed
 * container (webm or mp4 by webview) that must be decoded again before it can
 * be a 16 kHz WAV, which is another async step that can fail per webview; the
 * worklet hands over the raw samples the WAV needs, and stopping it is
 * synchronous, as the microphone-off paths require.
 */
import type { AudioContextLike } from './holdToTalk.mts';

/** Samples per posted block: ~21 ms at 48 kHz, the most a release can clip. */
const BLOCK = 1024;

const WORKLET = `
class SpeechTap extends AudioWorkletProcessor {
  constructor() { super(); this.block = new Float32Array(${BLOCK}); this.at = 0; }
  process(inputs) {
    const channel = inputs[0] && inputs[0][0];
    if (channel) {
      for (let i = 0; i < channel.length; i++) {
        this.block[this.at++] = channel[i];
        if (this.at === ${BLOCK}) {
          this.port.postMessage(this.block, [this.block.buffer]);
          this.block = new Float32Array(${BLOCK});
          this.at = 0;
        }
      }
    }
    return true;
  }
}
registerProcessor('speech-tap', SpeechTap);
`;

/** A fresh audio context with the recorder ready to load. */
export function newSpeechContext(): AudioContextLike {
  const Ctor = window.AudioContext ?? (window as unknown as { webkitAudioContext: typeof AudioContext }).webkitAudioContext;
  const ctx = new Ctor();
  return {
    get sampleRate() { return ctx.sampleRate; },
    destination: ctx.destination,
    resume: () => ctx.resume(),
    suspend: () => ctx.suspend(),
    async ready() {
      const url = URL.createObjectURL(new Blob([WORKLET], { type: 'text/javascript' }));
      try {
        await ctx.audioWorklet.addModule(url);
      } finally {
        URL.revokeObjectURL(url);
      }
    },
    createMediaStreamSource: (stream) => ctx.createMediaStreamSource(stream as unknown as MediaStream),
    createTap: () => new AudioWorkletNode(ctx, 'speech-tap', { numberOfInputs: 1, numberOfOutputs: 1, channelCount: 1 }) as unknown as ReturnType<AudioContextLike['createTap']>,
  };
}
