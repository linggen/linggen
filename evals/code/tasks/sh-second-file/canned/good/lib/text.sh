# count_noun N WORD -> "N WORD", with an s on WORD unless N is exactly 1.
count_noun() {
  if [ "$1" -ne 1 ]; then
    echo "$1 $2s"
  else
    echo "$1 $2"
  fi
}
