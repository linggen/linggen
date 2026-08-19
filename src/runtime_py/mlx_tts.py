# mlx_tts.py — resident TTS sidecar for the Linggen engine.
#
# Written to ~/.linggen/runtime/ by the engine at spawn time (the copy in
# the source tree is the truth; edits need an engine rebuild). Runs inside
# the `tts` venv on the managed Python runtime, which is the only place
# MLX is callable from — there is no Rust binding worth shipping.
#
# Protocol, JSON lines on stdio:
#   startup  ->  {"ready": true}                  (after the model loads)
#   request  <-  {"text": "...", "voice": "..."}
#   reply    ->  {"ok": true, "wav_b64": "...", "sr": 24000}
#            ->  {"ok": false, "error": "..."}    (the loop keeps serving)
#
# One request at a time by design — the engine holds a lock across
# write+read, and generation saturates the GPU anyway.

import base64
import io
import json
import os
import sys
import wave

# The protocol owns the REAL stdout; everything else that prints — model
# loading banners, tqdm, stray library chatter — is rerouted to stderr.
# Without this the first load_model() banner lands ahead of {"ready":true}
# and the engine reads garbage (hit live 2026-08-19).
_proto = os.fdopen(os.dup(1), "w")
os.dup2(2, 1)
sys.stdout = sys.stderr

import numpy as np


def emit(obj):
    _proto.write(json.dumps(obj) + "\n")
    _proto.flush()


def to_wav_bytes(chunks, sr):
    audio = np.concatenate(chunks) if chunks else np.zeros(1, dtype=np.float32)
    pcm = (np.clip(audio, -1.0, 1.0) * 32767.0).astype("<i2")
    buf = io.BytesIO()
    with wave.open(buf, "wb") as w:
        w.setnchannels(1)
        w.setsampwidth(2)
        w.setframerate(sr)
        w.writeframes(pcm.tobytes())
    return buf.getvalue()


def main():
    model_id = sys.argv[1]
    from mlx_audio.tts.utils import load_model

    model = load_model(model_id)
    emit({"ready": True})

    for line in sys.stdin:
        line = line.strip()
        if not line:
            continue
        try:
            req = json.loads(line)
            chunks, sr = [], 24000
            # stream=True purely to overlap decode and vocoder work; the
            # whole clip is still returned at once (the /api/tts contract).
            for res in model.generate(
                text=req["text"],
                voice=req["voice"],
                stream=True,
                streaming_interval=0.5,
            ):
                chunks.append(np.array(res.audio))
                sr = getattr(res, "sample_rate", sr)
            wav = to_wav_bytes(chunks, sr)
            emit({"ok": True, "wav_b64": base64.b64encode(wav).decode(), "sr": sr})
        except Exception as e:  # keep serving; the engine decides on fallback
            emit({"ok": False, "error": str(e)})


if __name__ == "__main__":
    main()
