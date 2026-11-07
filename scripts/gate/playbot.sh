# Release gate — 灵境 played on the skills being released (doc/test-design.md § 4). Sourced by gate.sh.
#
# The skills Linggen.app vendors (linggen-app/vendor/skills) and the ones the phone's 灵境 pack is built from
# (GATE_PHONE_SKILLS_REF, else the same commit) are each played by the play bot (skills/lingjing/tools/playbot,
# `just playbot`): a scratch checkout at that commit, scratch saves, headless and muted — never ~/.linggen or the
# player's Chrome. A hard finding or a layout-lint failure FAILs. Its vision review blocks green: the run's
# review.json starts "pending", shown as REVIEW until Claude has sorted every shot (sure pass / sure fail / unsure)
# for THAT run id and Hanli has answered each unsure one on 试玩台 (`tools/playbot/run.sh --ui`) — a sure fail
# FAILs without him. `just release-gate --verdict <run dir>` then re-reads it. The verdict is derived by the skills'
# tools/playbot/verdict.mjs (the same module the review page uses); a review of another run does not count. The phone's 灵境 tests run on assets built from the same skills
# (tool/lingjing_assets.sh, never committed): one that still skips is a GAP.

SKILLS_REPO="${GATE_SKILLS_REPO:-$WS/skills}"
APP_SKILLS="${GATE_APP_SKILLS:-$WS/linggen-app/vendor/skills}"
MOBILE_REPO="${GATE_MOBILE_REPO:-$WS/linggen-mobile}"
PLAYBOT="$SKILLS_REPO/lingjing/tools/playbot/run.sh"

# The skills' verdict module (tools/playbot/verdict.mjs, dependency-free): the checkout's, else origin/main's as a
# temp file (the shared checkout may lag), else none — and review_status falls back to the jq reading below.
verdict_module() {
  local m="$SKILLS_REPO/lingjing/tools/playbot/verdict.mjs" t
  [ -f "$m" ] && { echo "$m"; return; }
  command -v node >/dev/null 2>&1 || return
  t="$(cd "${TMPDIR:-/tmp}" && pwd -P)/gate-verdict-$$.mjs"
  git -C "$SKILLS_REPO" show origin/main:lingjing/tools/playbot/verdict.mjs >"$t" 2>/dev/null && [ -s "$t" ] && { echo "$t"; return; }
  rm -f "$t"
}

# review_status out run-id → PASS | FAIL | REVIEW, and a detail line, on stdout as "STATUS<TAB>detail".
# PASS: no hard finding, every shot answered by Claude, every unsure one answered by Hanli. REVIEW: "N items wait
# for Hanli: <shots>" or Claude not done. FAIL: a hard finding, a sure fail, or an unsure Hanli said no to.
review_status() {
  local mod line
  mod="$(verdict_module)"
  if [ -n "$mod" ] && line="$(node "$mod" "$1" "$2" --gate-run="${RUN:-}" 2>/dev/null)" && [ -n "$line" ]; then
    printf '%s\n' "$line"; return
  fi
  review_status_jq "$@"
}

# The reading before verdict.mjs (no node, or a skills checkout without it): pass/fail as the reviewer wrote it.
review_status_jq() {
  local out="$1" id="$2" f="$1/review.json"
  [ -f "$f" ] || { printf 'REVIEW\tREVIEW PENDING — no %s\n' "$f"; return; }
  # A two-tier review (notes with certainty) waits on Hanli's answers, which only verdict.mjs reads: never green here.
  if [ "$(jq '[.notes[]? | select(.certainty)] | length' "$f" 2>/dev/null || echo 0)" != 0 ]; then
    printf 'REVIEW\tREVIEW PENDING — a two-tier review needs verdict.mjs (node + skills origin/main), not found (%s)\n' "$f"; return
  fi
  local rid verdict notes answered total
  rid="$(jq -r '.run_id // ""' "$f" 2>/dev/null)"
  verdict="$(jq -r '.verdict // "pending"' "$f" 2>/dev/null)"
  notes="$(jq -r '(.notes // []) | length' "$f" 2>/dev/null)"
  answered="$(jq -r '.shots_answered // 0' "$f" 2>/dev/null)"
  total="$(jq -r '.shots_total // 0' "$f" 2>/dev/null)"
  if [ "$rid" != "$id" ]; then printf 'REVIEW\tREVIEW PENDING — review.json is for run %s, not %s\n' "${rid:-?}" "$id"
  elif [ "$verdict" = pass ] && [ "$answered" -ge "$total" ]; then printf 'PASS\t%s by %s, %s notes, %s/%s shots\n' "$verdict" "$(jq -r '.reviewer // "?"' "$f")" "$notes" "$answered" "$total"
  elif [ "$verdict" = pass ]; then printf 'REVIEW\tREVIEW PENDING — only %s/%s shots answered (%s)\n' "$answered" "$total" "$f"
  elif [ "$verdict" = fail ]; then printf 'FAIL\treviewer says fail: %s (%s)\n' "$(jq -r '[.notes[]? | (.answer // .)] | .[0:2] | join(" · ")' "$f" | cut -c1-120)" "$f"
  else printf 'REVIEW\tREVIEW PENDING — answer %s/checklist.md, write the verdict in review.json (run_id %s), then: just release-gate --verdict %s\n' "$out" "$id" "$RUN"; fi
}

# One play-bot run on a skills commit. label: app | phone | app+phone.
playbot_on() { # ref label
  local ref="$1" label="$2" out="$RUN/playbot-$2" id rc hard kinds played unplayable
  id="gate-$(basename "$RUN")-$label"
  say "Play bot: 灵境 on skills ${ref:0:12} ($label)"
  bash "$PLAYBOT" --ref "$ref" --out "$out" --run-id "$id" \
    ${GATE_PLAYBOT_SIZES:+--sizes "$GATE_PLAYBOT_SIZES"} ${GATE_PLAYBOT_FIGHTS:+--fights "$GATE_PLAYBOT_FIGHTS"} >"$out.log" 2>&1
  rc=$?
  if [ ! -f "$out/findings.json" ]; then record FAIL "host: play bot ($label)" "no report (exit $rc) — see $out.log"; return; fi
  hard="$(jq '[.findings[] | select(.hard)] | length' "$out/findings.json")"
  kinds="$(jq -r '[.findings[] | select(.hard) | .kind] | group_by(.) | map("\(.[0]) ×\(length)") | join(", ")' "$out/findings.json")"
  played="$(jq '.played | length' "$out/played.json")"
  unplayable="$(jq '.unplayable | length' "$out/played.json")"
  if [ "$hard" = 0 ] && [ "$rc" = 0 ]; then record PASS "host: play bot ($label)" "skills ${ref:0:12} · $played played"
  else record FAIL "host: play bot ($label)" "$hard hard: $kinds (see $out/findings.json)"; fi
  [ "$unplayable" = 0 ] || record WARN "host: play bot ($label) coverage" "$unplayable could not be played (see $out/played.json)"
  printf '%s\t%s\t%s\n' "$label" "$out" "$id" >>"$RUN/reviews.tsv"
  local st
  st="$(review_status "$out" "$id")"
  record "${st%%$'\t'*}" "review: play bot ($label)" "${st#*$'\t'}"
}

# The phone's 灵境 tests, on assets built from the released skills in a scratch mobile worktree.
phone_lingjing_tests() { # ref
  local ref="$1" mob="$RUN/mobile" sk="$RUN/skills-phone" flutter="${FLUTTER:-$(command -v flutter || echo "$HOME/dev/flutter/bin/flutter")}" json skipped failed passed
  [ -x "$flutter" ] || { record GAP "phone: 灵境 tests" "no flutter (FLUTTER=…)"; return; }
  [ -d "$MOBILE_REPO/.git" ] || [ -f "$MOBILE_REPO/.git" ] || { record GAP "phone: 灵境 tests" "no $MOBILE_REPO"; return; }
  say "Phone: 灵境 assets from skills ${ref:0:12}, then its tests"
  git -C "$MOBILE_REPO" worktree add -q --detach "$mob" "${GATE_MOBILE_REF:-HEAD}" >/dev/null 2>&1 || { record FAIL "phone: worktree" "git worktree add failed"; return; }
  git -C "$SKILLS_REPO" worktree add -q --detach "$sk" "$ref" >/dev/null 2>&1 || { record FAIL "phone: skills $ref" "git worktree add failed"; git -C "$MOBILE_REPO" worktree remove --force "$mob"; return; }
  if (cd "$mob" && tool/lingjing_assets.sh "$sk") >"$RUN/phone-assets.log" 2>&1; then record PASS "phone: 灵境 assets" "built from skills ${ref:0:12}"
  else record FAIL "phone: 灵境 assets" "tool/lingjing_assets.sh failed (see $RUN/phone-assets.log)"; fi
  json="$RUN/phone-lingjing-tests.json"
  (cd "$mob" && "$flutter" pub get >/dev/null 2>&1 && "$flutter" test --reporter json --concurrency="${GATE_FLUTTER_CONCURRENCY:-2}" $(cd "$mob" && ls test/lingjing_*_test.dart)) >"$json" 2>"$RUN/phone-lingjing-tests.err"
  skipped="$(grep '"type":"testDone"' "$json" | grep -c '"skipped":true')"
  failed="$(grep '"type":"testDone"' "$json" | grep -c '"result":"\(failure\|error\)"')"
  passed="$(grep '"type":"testDone"' "$json" | grep '"hidden":false' | grep '"skipped":false' | grep -c '"result":"success"')"
  if [ "$failed" != 0 ]; then record FAIL "phone: 灵境 tests" "$failed failed, $passed passed (see $json)"
  elif [ "$passed" = 0 ]; then record FAIL "phone: 灵境 tests" "none ran (see $RUN/phone-lingjing-tests.err)"
  else record PASS "phone: 灵境 tests" "$passed passed"; fi
  [ "$skipped" = 0 ] || record GAP "phone: 灵境 tests skipped" "$skipped skipped — the generated kit did not reach them (see $json)"
  git -C "$MOBILE_REPO" worktree remove --force "$mob" >/dev/null 2>&1
  git -C "$SKILLS_REPO" worktree remove --force "$sk" >/dev/null 2>&1
}

# The host step gate.sh runs: the phone's assets and tests first, then the play bot on each released skills commit.
playbot_checks() {
  local app phone
  app="$(git -C "$APP_SKILLS" rev-parse HEAD 2>/dev/null)"
  [ -n "$app" ] || { record FAIL "host: play bot" "no vendored skills at $APP_SKILLS"; return; }
  git -C "$SKILLS_REPO" cat-file -e "$app^{commit}" 2>/dev/null || git -C "$SKILLS_REPO" fetch -q origin
  phone="${GATE_PHONE_SKILLS_REF:-$app}"
  phone="$(git -C "$SKILLS_REPO" rev-parse "$phone^{commit}" 2>/dev/null || echo "$phone")"
  record INFO "host: released skills" "app ${app:0:12} · phone ${phone:0:12}$([ -n "${GATE_PHONE_SKILLS_REF:-}" ] || echo ' (same as the app; GATE_PHONE_SKILLS_REF to say otherwise)')"
  phone_lingjing_tests "$phone"
  if [ "$phone" = "$app" ]; then playbot_on "$app" app+phone
  else playbot_on "$app" app; playbot_on "$phone" phone; fi
}

# `--verdict <run dir>`: the play bot's reviews read again for that run, the table re-drawn, exit 0 only when green.
verdict_of() { # run-dir
  RUN="$1"; RESULTS="$RUN/results.tsv"
  [ -f "$RESULTS" ] && [ -f "$RUN/reviews.tsv" ] || { echo "release-gate: $RUN holds no play-bot reviews" >&2; exit 2; }
  local label out id st tmp
  while IFS=$'\t' read -r label out id; do
    st="$(review_status "$out" "$id")"
    tmp="$(mktemp)"
    awk -F'\t' -v OFS='\t' -v c="review: play bot ($label)" -v s="${st%%$'\t'*}" -v d="${st#*$'\t'}" '$2==c { $3=s; $4=d } { print }' "$RESULTS" >"$tmp" && mv "$tmp" "$RESULTS"
    # The table cuts the detail; which shots wait (or what failed) is said here in full.
    [ "${st%%$'\t'*}" = PASS ] || printf 'review: play bot (%s) — %s\n' "$label" "${st#*$'\t'}"
  done <"$RUN/reviews.tsv"
  PHASE=summary
  summary
}
