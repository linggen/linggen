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
for i in 1 2 3 4; do
  touch -t "20240101000$i" "$tmp/f$i"
done
sh "$here/bin/rotate.sh" "$tmp" 2
check rotate_keeps_two "2" "$(ls "$tmp" | wc -l | tr -d ' ')"
check disk_quiet "" "$(sh "$here/bin/diskwatch.sh" "$tmp" 100)"
rm -rf "$tmp"

[ "$fail" = 0 ] && echo "all passed"
exit "$fail"
