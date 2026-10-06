set positional-arguments

# Release gate on a clean Mac VM (doc/test-design.md § 4): --local | --draft "engine=… mem=… app=…" [--prev <vm>] — gate.sh --help
release-gate *ARGS:
    bash scripts/gate/gate.sh "$@"
