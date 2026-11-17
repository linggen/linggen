#!/bin/sh
# diskwatch.sh DIR MB: warn when DIR holds more than MB megabytes.
here=$(cd "$(dirname "$0")" && pwd)
. "$here/../lib/common.sh"

dir=$(folder_arg "$1")
limit=${2:-100}
used=$(du -sm "$dir" | cut -f1)
if [ "$used" -gt "$limit" ]; then
  echo "WARN $dir uses ${used}MB (limit ${limit}MB)"
fi
