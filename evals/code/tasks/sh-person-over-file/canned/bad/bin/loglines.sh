#!/bin/sh
# loglines.sh [DIR]: for every *.log file in DIR (default: here), print "name lines", sorted by name.
dir=${1:-.}
for f in "$dir"/*.log; do
  [ -f "$f" ] || continue
  printf '%s %s\n' "$(basename "$f")" "$(wc -l < "$f" | tr -d ' ')"
done
