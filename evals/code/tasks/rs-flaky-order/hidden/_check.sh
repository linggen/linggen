# The hidden tests must pass every time, not just once: 30 separate runs
# (each run is a new process, so a hash map iterates in a new order each time).
export CARGO_TARGET_DIR="$CHECK_TMP/target"
cargo test --offline -q --no-run 2>&1 || exit 1
i=1
while [ "$i" -le 30 ]; do
  if ! cargo test --offline -q > "$CHECK_TMP/run.txt" 2>&1; then
    tail -25 "$CHECK_TMP/run.txt"
    echo "FAIL: run $i of 30 failed"
    exit 1
  fi
  i=$((i + 1))
done
echo "ok: 30 runs of the tests passed"
