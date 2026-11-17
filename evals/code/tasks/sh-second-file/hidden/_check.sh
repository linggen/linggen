# Hidden check: the shared helper itself, then the visible script test.
fail=0
here=$(pwd)
. "$here/lib/text.sh"
check() {
  if [ "$2" != "$3" ]; then
    echo "FAIL $1: expected '$2' got '$3'"
    fail=1
  fi
}
check zero "0 files" "$(count_noun 0 file)"
check one "1 file" "$(count_noun 1 file)"
check two "2 files" "$(count_noun 2 file)"
check word "0 cats" "$(count_noun 0 cat)"
sh "$here/test.sh" || fail=1
exit "$fail"
