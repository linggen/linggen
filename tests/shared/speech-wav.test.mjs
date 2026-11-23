// The clip Yinyue (and any app page) is spoken to with, in /shared/hold-to-talk.js: WAV, 16 kHz, mono, 16-bit PCM.
// Run: node --test tests/shared/*.test.mjs
import { test } from 'node:test';
import assert from 'node:assert/strict';
import { encodeWav, MAX_SPEECH_SECONDS, SPEECH_RATE, toBase64, toSpeechSamples } from '../../shared/hold-to-talk.js';

const ramp = (n) => Float32Array.from({ length: n }, (_, i) => (i / n) * 2 - 1);

test('the clip is a 16 kHz mono 16-bit PCM WAV: header and length', () => {
  const wav = encodeWav(Float32Array.from([0, 1, -1, 0.5]));
  const view = new DataView(wav.buffer);
  const text = (at, n) => String.fromCharCode(...wav.subarray(at, at + n));
  assert.equal(wav.length, 44 + 4 * 2);
  assert.equal(text(0, 4), 'RIFF');
  assert.equal(view.getUint32(4, true), wav.length - 8);
  assert.equal(text(8, 8), 'WAVEfmt ');
  assert.equal(view.getUint16(20, true), 1, 'PCM');
  assert.equal(view.getUint16(22, true), 1, 'mono');
  assert.equal(view.getUint32(24, true), 16_000);
  assert.equal(view.getUint32(28, true), 32_000);
  assert.equal(view.getUint16(34, true), 16, 'bits');
  assert.equal(text(36, 4), 'data');
  assert.equal(view.getUint32(40, true), 8);
  assert.deepEqual([0, 1, 2, 3].map((i) => view.getInt16(44 + i * 2, true)), [0, 32767, -32768, Math.round(0.5 * 32767)]);
});

test('samples beyond full scale are clipped, not wrapped', () => {
  const wav = encodeWav(Float32Array.from([3, -3]));
  const view = new DataView(wav.buffer);
  assert.equal(view.getInt16(44, true), 32767);
  assert.equal(view.getInt16(46, true), -32768);
});

test('a 48 kHz clip comes down to a third of its samples, its level kept', () => {
  const flat = new Float32Array(48_000).fill(0.25);
  const out = toSpeechSamples(flat, 48_000);
  assert.equal(out.length, 16_000);
  assert.ok(out.every((s) => Math.abs(s - 0.25) < 1e-6));
});

test('down-sampling averages what it covers (44.1 kHz: a fractional ratio)', () => {
  const out = toSpeechSamples(ramp(44_100), 44_100);
  assert.equal(out.length, 16_000);
  for (let i = 1; i < out.length; i++) assert.ok(out[i] > out[i - 1], 'a ramp stays a ramp');
});

test('a clip already at 16 kHz is unchanged; a slower one is stretched', () => {
  const same = ramp(1_000);
  assert.deepEqual(toSpeechSamples(same, 16_000), same);
  assert.equal(toSpeechSamples(ramp(800), 8_000).length, 1_600);
});

test('a clip is cut at 60 seconds', () => {
  const out = toSpeechSamples(new Float32Array(48_000 * 75), 48_000);
  assert.equal(out.length, SPEECH_RATE * MAX_SPEECH_SECONDS);
});

test('base64 of a clip-sized array does not overflow the stack and round-trips', () => {
  const bytes = Uint8Array.from({ length: 1_920_044 }, (_, i) => i % 251);
  assert.deepEqual(new Uint8Array(Buffer.from(toBase64(bytes), 'base64')), bytes);
});
