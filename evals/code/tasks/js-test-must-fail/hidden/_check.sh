# Hidden check, in three steps:
#  1. the project's own tests (the model's new test included) pass on its code;
#  2. the limiter behaves (hidden_cases.cjs);
#  3. the project's tests, run against the ORIGINAL limiter, must fail somewhere:
#     a new test that cannot fail does not count.
fail=0
if ! node --test > "$CHECK_TMP/own.txt" 2>&1; then
  tail -30 "$CHECK_TMP/own.txt"
  echo "FAIL: the project's own tests do not pass"
  exit 1
fi
node --test hidden_cases.cjs || fail=1

old="$CHECK_TMP/old"
mkdir -p "$old"
cp -R . "$old"
cp orig/limiter.js.txt "$old/lib/limiter.js"
if (cd "$old" && node --test > "$CHECK_TMP/old.txt" 2>&1); then
  echo "FAIL: every test passes on the original limiter, so the new test cannot fail"
  fail=1
else
  echo "ok: a test fails on the original limiter"
fi
exit "$fail"
