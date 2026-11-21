# mlx_voice.py — resident voice sidecar for the Linggen engine: her voice
# (speech out) and her hearing (speech in), one process on one runtime.
#
# Written to ~/.linggen/runtime/ by the engine at spawn time (the copy in
# the source tree is the truth; edits need an engine rebuild). Runs inside
# the `tts` venv on the managed Python runtime, which is the only place
# MLX is callable from — there is no Rust binding worth shipping.
#
# argv: <tts model id> <hearing model id>
#
# Protocol, JSON lines on stdio:
#   startup  ->  {"ready": true}       (after the voice model loads; the
#                                       hearing model loads on its first use)
#   speak    <-  {"text": "...", "voice": "...", "lang": "english"}
#                (no "op"; lang: auto | chinese | english; absent = auto.
#                The engine sends one short clause per request and names
#                the language whenever the clause's script or the user's
#                language setting makes it known.)
#   reply    ->  {"ok": true, "wav_b64": "...", "sr": 24000}
#   hear     <-  {"op": "hear", "pcm_b64": "..."}
#                (16 kHz mono 16-bit little-endian samples, nothing else)
#   reply    ->  {"ok": true, "text": "...", "language": "zh"}
#   either   ->  {"ok": false, "error": "..."}    (the loop keeps serving)
#
# One request at a time by design — the engine holds a lock across
# write+read, and generation saturates the GPU anyway.
#
# What is heard stays in memory: the samples arrive on stdin and are handed
# to the model as an array. No file is written, and this script has no
# network code — the hearing model is read from the local cache by path
# (local_files_only), never by a hub lookup. A test holds both
# (`stt_mlx::the_sidecar_script_keeps_what_it_hears_to_itself`).

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


def hear(req, state):
    """Words from 16 kHz mono PCM16 samples. The model loads on first use."""
    if state.get("stt") is None:
        from pathlib import Path

        from huggingface_hub import snapshot_download
        from mlx_audio.stt.utils import load_model as load_stt

        path = snapshot_download(state["stt_id"], local_files_only=True)
        # A Path, not a str: mlx-audio reads the repo's name off the cache
        # path (its `hub` part) when it picks the model class.
        state["stt"] = load_stt(Path(path))
    pcm = np.frombuffer(base64.b64decode(req["pcm_b64"]), dtype="<i2")
    audio = pcm.astype(np.float32) / 32768.0
    out = state["stt"].generate(audio)
    return {
        "ok": True,
        "text": (out.text or "").strip(),
        "language": getattr(out, "language", None) or "",
    }


def main():
    model_id = sys.argv[1]
    state = {"stt": None, "stt_id": sys.argv[2]}
    from mlx_audio.tts.utils import load_model

    model = load_model(model_id)
    emit({"ready": True})

    for line in sys.stdin:
        line = line.strip()
        if not line:
            continue
        try:
            req = json.loads(line)
            if req.get("op") == "hear":
                emit(hear(req, state))
                continue
            chunks, sr = [], 24000
            # Cap generation by the text's plausible audio length (speech
            # tokens run at 12Hz; ~0.5s of audio per char is already twice
            # a slow zh reading) so an autoregressive runaway can never
            # stall the clip until the engine's timeout.
            # The floor (5 s) is for the shortest clauses the engine sends.
            max_tokens = min(max(int(len(req["text"]) * 0.5 * 12), 60), 1800)
            # stream=True purely to overlap decode and vocoder work; the
            # whole clip is still returned at once (the /api/tts contract).
            for res in model.generate(
                text=req["text"],
                voice=req["voice"],
                lang_code=req.get("lang", "auto"),
                stream=True,
                streaming_interval=0.5,
                max_tokens=max_tokens,
            ):
                chunks.append(np.array(res.audio))
                sr = getattr(res, "sample_rate", sr)
            wav = to_wav_bytes(chunks, sr)
            emit({"ok": True, "wav_b64": base64.b64encode(wav).decode(), "sr": sr})
        except Exception as e:  # keep serving; the engine decides on fallback
            emit({"ok": False, "error": str(e)})


if __name__ == "__main__":
    main()
