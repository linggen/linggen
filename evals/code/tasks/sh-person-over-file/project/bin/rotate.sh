#!/bin/sh
# rotate.sh DIR N: keep the newest N files in DIR, delete the rest.
here=$(cd "$(dirname "$0")" && pwd)
. "$here/../lib/common.sh"

dir=$(folder_arg "$1")
keep=${2:-5}
ls -t "$dir" | tail -n +"$((keep + 1))" | while read -r name; do
  rm -f "$dir/$name"
done
