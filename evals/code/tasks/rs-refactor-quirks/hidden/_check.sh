# 1. the behaviour: the original tests plus the cases they leave out;
# 2. the refactor happened: the three fees now share code, so the file is at
#    at least a sixth smaller (code lines only: no comments, no blank lines, no
#    `mod tests;`). The original has 39.
export CARGO_TARGET_DIR="$CHECK_TMP/target"
fail=0
cargo test --offline -q || fail=1
code=$(grep -v '^[[:space:]]*//' src/lib.rs | grep -v '^[[:space:]]*$' | grep -v 'mod tests' | grep -v 'cfg(test)' | wc -l | tr -d ' ')
if [ "$code" -gt 32 ]; then
  echo "FAIL: src/lib.rs still has $code code lines (limit 32): the three fees do not share code"
  fail=1
fi
for name in fee_standard fee_express fee_freight; do
  grep -q "pub fn $name" src/lib.rs || { echo "FAIL: pub fn $name is gone"; fail=1; }
done
exit "$fail"
