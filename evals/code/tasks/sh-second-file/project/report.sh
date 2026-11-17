#!/bin/sh
# Print a one-line summary of the files in a directory.
here=$(cd "$(dirname "$0")" && pwd)
. "$here/lib/text.sh"

dir=${1:-.}
n=$(find "$dir" -type f | wc -l | tr -d ' ')
echo "Found $(count_noun "$n" file) in $dir"
