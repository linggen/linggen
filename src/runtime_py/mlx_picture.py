# mlx_picture.py — one-shot picture maker for the Linggen engine.
#
# Written to ~/.linggen/runtime/ by the engine at spawn time (the copy in
# the source tree is the truth; edits need an engine rebuild). Runs inside
# the `pictures` venv on the managed Python runtime — MLX is only callable
# from Python. One process per picture by design: the model loads in
# under a second from the HF cache and a render holds 7–14 GB of unified
# memory, which must go back to the machine the moment the picture is
# saved. Nothing here stays resident.
#
# Protocol: one JSON object on stdin, one JSON line on stdout.
#   request  {"model": "...", "prompt": "...", "out": "/abs/path.png",
#             "width": 512, "height": 512, "seed": 7,
#             "reference": "/abs/plate.jpg" | null}
#   reply    {"ok": true, "path": "...", "seconds": 11.2, "seed": 7}
#            {"ok": false, "error": "..."}
#
# With a reference the edit variant runs (it keeps the reference's pose
# and shape); without one, plain text-to-image.

import json
import os
import sys
import time

# The protocol owns the REAL stdout; library banners and tqdm bars go to
# stderr, which the engine streams into its debug log.
_proto = os.fdopen(os.dup(1), "w")
os.dup2(2, 1)
sys.stdout = sys.stderr


def emit(obj):
    _proto.write(json.dumps(obj) + "\n")
    _proto.flush()


def main():
    req = json.loads(sys.stdin.read() or "{}")
    from huggingface_hub import snapshot_download
    from mflux.models.common.config.model_config import ModelConfig

    started = time.time()
    path = snapshot_download(req["model"])
    width, height = int(req.get("width", 512)), int(req.get("height", 512))
    seed = int(req.get("seed", 0))
    reference = req.get("reference")
    steps = int(req.get("steps", 4))
    cfg = ModelConfig.flux2_klein_4b()

    if reference:
        from mflux.models.flux2.variants.edit.flux2_klein_edit import Flux2KleinEdit

        model = Flux2KleinEdit(model_config=cfg, model_path=path)
        image = model.generate_image(
            seed=seed, prompt=req["prompt"], num_inference_steps=steps,
            height=height, width=width, guidance=1.0, image_paths=[reference],
        )
    else:
        from mflux.models.flux2.variants.txt2img.flux2_klein import Flux2Klein

        model = Flux2Klein(model_config=cfg, model_path=path)
        image = model.generate_image(
            seed=seed, prompt=req["prompt"], num_inference_steps=steps,
            height=height, width=width, guidance=1.0,
        )

    out = req["out"]
    os.makedirs(os.path.dirname(out), exist_ok=True)
    image.save(path=out)
    emit({"ok": True, "path": out, "seconds": round(time.time() - started, 1), "seed": seed})


if __name__ == "__main__":
    try:
        main()
    except Exception as e:  # one line back, never a traceback on the protocol
        emit({"ok": False, "error": f"{type(e).__name__}: {e}"})
        sys.exit(1)
