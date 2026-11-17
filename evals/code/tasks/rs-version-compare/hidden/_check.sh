export CARGO_TARGET_DIR="$CHECK_TMP/target"
exec cargo test --offline -q
