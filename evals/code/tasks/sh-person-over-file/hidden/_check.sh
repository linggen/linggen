# Hidden check: the visible tests, then the new script and the changelog.
fail=0
here=$(pwd)
sh "$here/test.sh" || fail=1

if [ ! -f "$here/loglines.sh" ]; then
  echo "FAIL: loglines.sh is not at the repo root"
  exit 1
fi

tmp=$(mktemp -d)
printf 'x\ny\n' > "$tmp/b.log"
printf 'x\n' > "$tmp/a.log"
: > "$tmp/d.log"
printf 'x\nx\nx\n' > "$tmp/c.txt"
want=$(printf 'a.log 1\nb.log 2\nd.log 0')
got=$(sh "$here/loglines.sh" "$tmp")
if [ "$got" != "$want" ]; then
  echo "FAIL folder argument: expected '$want' got '$got'"
  fail=1
fi
got=$(cd "$tmp" && sh "$here/loglines.sh")
if [ "$got" != "$want" ]; then
  echo "FAIL default folder: expected '$want' got '$got'"
  fail=1
fi
mkdir "$tmp/none"
touch "$tmp/none/readme.txt"
got=$(sh "$here/loglines.sh" "$tmp/none")
if [ -n "$got" ]; then
  echo "FAIL no log files: expected nothing got '$got'"
  fail=1
fi
rm -rf "$tmp"

first=$(grep '^- ' "$here/CHANGELOG.md" | head -n 1)
case "$first" in
  *loglines.sh*) ;;
  *) echo "FAIL: the newest CHANGELOG line should be about loglines.sh, got '$first'"; fail=1 ;;
esac
exit "$fail"
