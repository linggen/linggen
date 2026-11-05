set positional-arguments

# Release gate on a clean Mac VM (doc/test-design.md § 4): --local | --draft "engine=… mem=… app=…" [--prev <vm>] — gate.sh --help
release-gate *ARGS:
    bash scripts/gate/gate.sh "$@"

# 灵境 played by its play bot on a skills ref or an install dir (scratch saves, headless): [--ref <ref> | --install <dir>] [--sizes 430x932,375x812,1280x820] [--fights N] [--selftest] — ../skills/lingjing/tools/playbot/run.sh --help
playbot *ARGS:
    bash "{{justfile_directory()}}/../skills/lingjing/tools/playbot/run.sh" "$@"
