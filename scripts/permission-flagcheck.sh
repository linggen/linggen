#!/bin/sh
# How each grep / sed / awk / jq / rg on this system reads the flag shapes the
# permission gate makes assumptions about (src/engine/permission/
# pattern_text.rs and scripts.rs). Harmless: works in a scratch folder on
# files it creates; payloads only print a marker.
#
#   sh scripts/permission-flagcheck.sh mac
#   docker run --rm -v "$PWD/scripts/permission-flagcheck.sh:/c.sh:ro" ubuntu:24.04 \
#     sh -c 'apt-get update -qq && apt-get install -y -qq gawk jq ripgrep busybox original-awk >/dev/null; sh /c.sh ubuntu'
#   docker run --rm -v "$PWD/scripts/permission-flagcheck.sh:/c.sh:ro" alpine \
#     sh -c 'apk add --no-cache gawk jq ripgrep >/dev/null; sh /c.sh alpine'
#
# Read the output against the tables: a flag the gate lists as taking a value
# must never print SWITCH, and for awk a flag whose next word the gate takes
# as a value must never print PROGRAM RAN. Run it on all three before a flag
# is added to a table (last full run 2026-11-14: BSD grep 2.6.0 / sed, awk
# 20200816 and 20231127, GNU grep 3.11, GNU sed 4.9, gawk 5.2-5.3, mawk 1.3.4,
# busybox 1.36-1.37, jq 1.7-1.8.2, rg 14.1-15.1).
L=${1:-here}
W=$(mktemp -d "${TMPDIR:-/tmp}/flagcheck.XXXXXX") && cd "$W" || exit 1
printf 'a\nfoo\nb\n' > f
printf 'SECRET-foo\n' > foo
printf '{"k":1}\n' > f.json
say() { printf '%s|%s|%s|%s\n' "$L" "$1" "$2" "$3"; }
has() { command -v "$1" >/dev/null 2>&1; }

# --- grep: does the flag take the next word as its value? --------------------
for G in grep ggrep "busybox grep"; do
  has ${G%% *} || continue
  [ "$G" = "busybox grep" ] && ! busybox grep --help >/dev/null 2>&1 && continue
  v=$($G --version 2>&1 | head -1); [ -n "$v" ] || v=$($G --help 2>&1 | head -1)
  say "$G" version "$v"
  for fl in -A -B -C -m -d -D --after-context --before-context --context --max-count \
            --directories --devices --label --include --exclude --exclude-dir \
            --exclude-from --binary-files --color; do
    out=$($G $fl foo f </dev/null 2>/dev/null)
    if printf '%s\n' "$out" | /usr/bin/env grep -q '^foo$' 2>/dev/null || printf '%s\n' "$out" | grep -q '^foo$'; then
      say "$G" "$fl foo f" "SWITCH (next word was the pattern)"
    else
      say "$G" "$fl foo f" "takes-value-or-error"
    fi
  done
done

# --- sed ----------------------------------------------------------------------
for S in sed gsed "busybox sed"; do
  has ${S%% *} || continue
  [ "$S" = "busybox sed" ] && ! busybox sed --help >/dev/null 2>&1 && continue
  v=$($S --version 2>&1 | head -1)
  say "$S" version "$v"
  # short/long flags the gate reads as switches: is the next word still the script?
  for fl in -l -s -E -r -z -u --debug --posix --sandbox --line-length; do
    out=$($S -n $fl p f </dev/null 2>/dev/null | tr '\0' '\n')
    if printf '%s\n' "$out" | grep -q '^foo$'; then say "$S" "-n $fl p f" "SWITCH (p was the script)"
    else say "$S" "-n $fl p f" "takes-value-or-error"; fi
  done
  # bare -i: is the next word a suffix (BSD) or the script (GNU)?
  cp f g; $S -i '' 's/foo/CHANGED/' g </dev/null 2>/dev/null
  if grep -q CHANGED g; then say "$S" "-i '' SCRIPT g" "BSD reading: '' is the suffix, next word the script"
  else say "$S" "-i '' SCRIPT g" "GNU reading: '' is the script, next word a file (g unchanged)"; fi
  cp f g; $S -n -i '' g </dev/null 2>/dev/null; say "$S" "-n -i '' g" "g is now $(wc -c < g | tr -d ' ') bytes (was 8)"
  cp f g; $S -i 's/foo/CHANGED/' g </dev/null 2>/dev/null
  if grep -q CHANGED g; then say "$S" "-i SCRIPT g" "script taken (GNU form works)"; else say "$S" "-i SCRIPT g" "not edited (BSD: SCRIPT was the suffix)"; fi
  # commands that run or write
  out=$(printf 'x\n' | $S '1e echo MARK-E' 2>/dev/null); case "$out" in *MARK-E*) say "$S" "1e cmd" "RUNS a command";; *) say "$S" "1e cmd" "no e command";; esac
  rm -f wout; $S -n 'w wout' f </dev/null 2>/dev/null; [ -s wout ] && say "$S" "w file" "writes the file" || say "$S" "w file" "no write"
done

# --- awk ----------------------------------------------------------------------
P='BEGIN{print "X-RAN"}'
for A in awk gawk mawk "busybox awk" original-awk; do
  has ${A%% *} || continue
  [ "$A" = "busybox awk" ] && ! busybox awk 'BEGIN{}' >/dev/null 2>&1 && continue
  v=$($A --version 2>&1 | head -1); case "$v" in *sage*|*nrecognized*|'') v=$($A -version 2>&1 | head -1);; esac
  [ -n "$v" ] || v=$($A -W version 2>&1 | head -1)
  say "$A" version "$v"
  for fl in -F -v -i -l --assign --field-separator --include --load --source -e; do
    out=$($A $fl "$P" f </dev/null 2>/dev/null)
    case "$out" in *X-RAN*) say "$A" "$fl PROGRAM f" "PROGRAM RAN (flag took no value, or it is the program flag)";;
                   *) say "$A" "$fl PROGRAM f" "did not run (flag took it as a value, or error)";; esac
  done
done

# --- jq / rg ------------------------------------------------------------------
if has jq; then
  say jq version "$(jq --version 2>&1)"
  for fl in -L --indent; do
    out=$(jq $fl . f.json </dev/null 2>/dev/null); case "$out" in *'"k"'*) say jq "$fl . f.json" "SWITCH (. was the program)";; *) say jq "$fl . f.json" "takes-value-or-error";; esac
  done
  for fl in --arg --argjson --slurpfile --rawfile; do
    out=$(jq $fl a . f.json </dev/null 2>/dev/null); case "$out" in *'"k"'*) say jq "$fl a . f.json" "takes ONE value only";; *) say jq "$fl a . f.json" "takes two values (or error)";; esac
  done
fi
if has rg; then
  say rg version "$(rg --version | head -1)"
  for fl in -A -B -C -m -g -t -T -j -M -E -r -d --glob --iglob --type --type-not --type-add \
            --max-count --max-depth --pre --pre-glob --ignore-file --encoding --replace --context \
            --after-context --before-context --threads --max-columns --max-filesize --sort --sortr \
            --color --colors --engine --path-separator; do
    out=$(rg --no-config -N $fl foo f </dev/null 2>/dev/null)
    if printf '%s\n' "$out" | grep -q '^foo$'; then say rg "$fl foo f" "SWITCH (next word was the pattern)"; else say rg "$fl foo f" "takes-value-or-error"; fi
  done
fi
cd / && rm -rf "$W"
