# 1. the behaviour: the original tests plus the cases they leave out;
# 2. the refactor happened: the step arithmetic (a division) is written once
#    or twice, not once per service. The original has it three times. Comments
#    are not counted, so a no-op or a rename passes neither this nor the ask.
export CARGO_TARGET_DIR="$CHECK_TMP/target"
fail=0
cargo test --offline -q || fail=1
divs=$(sed 's,//.*,,' src/lib.rs | grep -cE '[A-Za-z0-9_)][[:space:]]*/[[:space:]]*[A-Za-z0-9_(]')
if [ "$divs" -gt 2 ]; then
  echo "FAIL: src/lib.rs still does the step division on $divs lines (limit 2): the three fees do not share code"
  fail=1
fi
exit "$fail"
