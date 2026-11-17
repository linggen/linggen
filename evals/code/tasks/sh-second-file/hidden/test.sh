#!/bin/sh
# Tiny test script: prints FAIL lines, exits 1 if any.
here=$(cd "$(dirname "$0")" && pwd)
fail=0
check() {
  if [ "$2" != "$3" ]; then
    echo "FAIL $1: expected '$2' got '$3'"
    fail=1
  fi
}

tmp=$(mktemp -d)
mkdir "$tmp/empty" "$tmp/one" "$tmp/two"
touch "$tmp/one/a" "$tmp/two/a" "$tmp/two/b"

check one "Found 1 file in $tmp/one" "$(sh "$here/report.sh" "$tmp/one")"
check two "Found 2 files in $tmp/two" "$(sh "$here/report.sh" "$tmp/two")"
check empty "Found 0 files in $tmp/empty" "$(sh "$here/report.sh" "$tmp/empty")"

rm -rf "$tmp"
[ "$fail" = 0 ] && echo "all passed"
exit "$fail"
