# Release gate — upgrade an old user (a clone of linggen-prev) to the release
# in the mirror, then the refusal and rollback cases (doc/test-design.md § 4).
# Env from the host: MIRROR, MODEL, DREAM_SHA (the release's built-in dream
# mission), and expect.env.
. ./common.sh
. ./expect.env

LING="$(ling_bin)"; MEM="$(mem_bin)"
PREV_LING="$(ver_of "$LING")"; PREV_MEM="$(ver_of "$MEM")"; PREV_APP="$(app_version)"
info "upgrade: from" "ling $PREV_LING, ling-mem $PREV_MEM, app ${PREV_APP:-none}"

# The old user, running: engine (fixtures config → the fake model) + ling-mem.
start_engine 90 || fail "upgrade: old engine runs" "no health"
mem_health 30 || start_mem 60
rows_before="$("$MEM" stats 2>/dev/null | jq -r '.total // empty')"
sessions_before="$(ls "$LG/sessions" 2>/dev/null | wc -l | tr -d ' ')"
info "upgrade: old state" "$rows_before memory rows, $sessions_before sessions"

tcc_watch_start

# ── Engine ─────────────────────────────────────────────────────────────────
if "$LING" update --help 2>/dev/null | grep -q -- '--rollback'; then
  LINGGEN_RELEASE_BASE="$GOOD" "$LING" update >"$OUT/ling-update.log" 2>&1
  check "ling update (from $PREV_LING)" $? "$(tail -3 "$OUT/ling-update.log")"
else
  gap "ling update (from $PREV_LING)" "the old binary predates LINGGEN_RELEASE_BASE — upgraded by re-running install.sh instead"
  curl -fsSL "$GOOD/install.sh" | LINGGEN_RELEASE_BASE="$GOOD" bash >"$OUT/install.log" 2>&1
  check "install.sh over a running install" $? "$(tail -3 "$OUT/install.log")"
fi
check_installed "upgrade: ling is the release" "$LING" "$LING_VERSION" "$LING_SHA"

# ── ling-mem ───────────────────────────────────────────────────────────────
if [ "$(ver_of "$MEM")" != "$MEM_VERSION" ] || [ "$(sha "$MEM")" != "$MEM_SHA" ]; then
  if "$MEM" upgrade --help 2>/dev/null | grep -q -- '--rollback'; then
    LINGGEN_RELEASE_BASE="$GOOD" "$MEM" upgrade --yes >"$OUT/mem-upgrade.log" 2>&1
    check "ling-mem upgrade --yes (from $PREV_MEM)" $? "$(tail -3 "$OUT/mem-upgrade.log")"
  else
    gap "ling-mem upgrade --yes (from $PREV_MEM)" "the old binary predates LINGGEN_RELEASE_BASE — upgraded with install-bin.sh"
    curl -fsSL "$GOOD/install-bin.sh" | LINGGEN_RELEASE_BASE="$GOOD" bash -s -- --version "^${MEM_VERSION%%.*}" >"$OUT/install-bin.log" 2>&1
    check "install-bin.sh from the mirror" $? "$(tail -3 "$OUT/install-bin.log")"
  fi
else
  info "ling-mem: upgraded by install.sh" "install.sh runs install-bin.sh (^1)"
fi
check_installed "upgrade: ling-mem is the release" "$MEM" "$MEM_VERSION" "$MEM_SHA"

# ── The upgraded engine on the old home ────────────────────────────────────
v="$(mem_version)"
[ "$v" = "$MEM_VERSION" ] && pass "ling-mem daemon runs the new binary" "v$v" \
  || warn "ling-mem daemon runs the new binary" "9528 still answers v${v:-none} after the binary swap — restarting it by hand"
stop_engine
[ "$v" = "$MEM_VERSION" ] || { stop_mem; start_mem 60; }
start_engine 90
check "upgrade: new engine starts on the old home" $? "no health — $(tail -3 "$LG/ling.log" 2>/dev/null)"
v="$(mem_version)"
[ "$v" = "$MEM_VERSION" ]; check "upgrade: 9528 is the new ling-mem" $? "v${v:-none}" "v$v"

"$LING" status >"$OUT/status.txt" 2>&1
grep -q 'fake-ling' "$OUT/status.txt" || grep -qiE 'models?[^0-9]*[1-9]' "$OUT/status.txt"
check "upgrade: old config loads" $? "$(head -c 200 "$OUT/status.txt")"
sessions_after="$(ls "$LG/sessions" 2>/dev/null | wc -l | tr -d ' ')"
[ "$sessions_after" -ge "$sessions_before" ] && grep -q 'Harbor table' "$LG"/sessions/*/session.yaml
check "upgrade: old sessions kept" $? "$sessions_after now, $sessions_before before"
for f in "$LG"/sessions/sess-17900000*/messages.jsonl; do jq -e . "$f" >/dev/null 2>&1 || { fail "upgrade: old session rows parse" "$f"; break; }; done
chat_turn 90
check "upgrade: a chat turn on the old config" $? "$CHAT_DETAIL" "$CHAT_DETAIL"

[ -f "$LG/missions/table-check/mission.md" ]
check "upgrade: user mission kept" $? "missions/table-check gone"
dream="$(sha "$LG/missions/dream/mission.md")"
if [ "$dream" = "$DREAM_SHA" ]; then pass "upgrade: built-in dream mission is this release's" ""
else info "upgrade: built-in dream mission is this release's" "kept the earlier copy (missions are install-once, cli/init.rs)"; fi

# ── The store ──────────────────────────────────────────────────────────────
"$MEM" stats >"$OUT/stats.json" 2>&1
rows_after="$(jq -r '.total // empty' "$OUT/stats.json")"; schema="$(jq -r '.schema_version // empty' "$OUT/stats.json")"
[ -n "$rows_after" ] && [ "$rows_after" = "$rows_before" ]
check "store: opens with every row" $? "rows $rows_before → ${rows_after:-?} ($(head -c 120 "$OUT/stats.json"))" "$rows_after rows, schema $schema"
if [ -n "$schema" ] && [ "$schema" -lt 2 ] 2>/dev/null; then
  "$MEM" apply-schema --yes >"$OUT/apply-schema.log" 2>&1
  check "store: apply-schema --yes" $? "$(tail -2 "$OUT/apply-schema.log")"
  "$MEM" stats >"$OUT/stats2.json" 2>&1
  [ "$(jq -r .total "$OUT/stats2.json")" = "$rows_before" ] && [ "$(jq -r .schema_version "$OUT/stats2.json")" = 2 ]
  check "store: migrated to schema 2, rows kept" $? "$(head -c 160 "$OUT/stats2.json")"
fi
"$MEM" session-start --format json >"$OUT/session-start.json" 2>&1
check "store: session_start answers" $? "$(head -c 160 "$OUT/session-start.json")"
"$MEM" list --limit 50 >"$OUT/list.json" 2>&1 && grep -q 'Lisbon' "$OUT/list.json"
check "store: the old rows read back" $? "$(head -c 160 "$OUT/list.json")"
stop_engine

# ── Refusal: every sha256 in the mirror wrong ──────────────────────────────
before="$(sha "$LING")"
LINGGEN_RELEASE_BASE="$BAD" "$LING" update >"$OUT/ling-update-bad.log" 2>&1
rc=$?
[ "$rc" != 0 ] && [ "$(sha "$LING")" = "$before" ] && "$LING" --version >/dev/null 2>&1
check "ling update: a wrong sha256 is refused, old binary runs" $? "exit $rc — $(tail -2 "$OUT/ling-update-bad.log")" "$(grep -m1 -i 'integrity\|sha' "$OUT/ling-update-bad.log")"

before="$(sha "$MEM")"
LINGGEN_RELEASE_BASE="$BAD" "$MEM" upgrade --yes --force >"$OUT/mem-upgrade-bad.log" 2>&1
rc=$?
[ "$rc" != 0 ] && [ "$(sha "$MEM")" = "$before" ] && "$MEM" --version >/dev/null 2>&1 && mem_health 20
check "ling-mem upgrade: a wrong sha256 is refused, old binary runs" $? "exit $rc — $(tail -2 "$OUT/mem-upgrade-bad.log")"

# ── Rollback ───────────────────────────────────────────────────────────────
if [ ! -f "$LING.prev" ]; then # install.sh path: give `ling update` one real swap first
  LINGGEN_RELEASE_BASE="$RELABEL" "$LING" update >"$OUT/ling-update-relabel.log" 2>&1
  check "ling update: installs and keeps ling.prev" $? "$(tail -2 "$OUT/ling-update-relabel.log")"
fi
a="$(sha "$LING")"; p="$(sha "$LING.prev")"
"$LING" update --rollback >"$OUT/ling-rollback.log" 2>&1
rc=$?
[ "$rc" = 0 ] && [ "$(sha "$LING")" = "$p" ] && [ "$(sha "$LING.prev")" = "$a" ]
check "ling update --rollback swaps to ling.prev" $? "exit $rc — $(tail -2 "$OUT/ling-rollback.log")" "now $(ver_of "$LING")"
"$LING" update --rollback >>"$OUT/ling-rollback.log" 2>&1
[ "$(sha "$LING")" = "$a" ]; check "ling update --rollback again returns" $? "sha ${a:0:12} not back"

if [ ! -f "$MEM.prev" ]; then
  LINGGEN_RELEASE_BASE="$GOOD" "$MEM" upgrade --yes --force >"$OUT/mem-upgrade-force.log" 2>&1
  check "ling-mem upgrade --force keeps ling-mem.prev" $? "$(tail -2 "$OUT/mem-upgrade-force.log")"
fi
a="$(sha "$MEM")"; p="$(sha "$MEM.prev")"
"$MEM" upgrade --rollback >"$OUT/mem-rollback.log" 2>&1
rc=$?
[ "$rc" = 0 ] && [ "$(sha "$MEM")" = "$p" ] && mem_health 30
check "ling-mem upgrade --rollback swaps to ling-mem.prev" $? "exit $rc — $(tail -2 "$OUT/mem-rollback.log")" "now $(ver_of "$MEM")"
"$MEM" upgrade --rollback >>"$OUT/mem-rollback.log" 2>&1
[ "$(sha "$MEM")" = "$a" ] && mem_health 30; check "ling-mem upgrade --rollback again returns" $? "sha ${a:0:12} not back"

# ── App ────────────────────────────────────────────────────────────────────
if [ -n "$APP_VERSION" ]; then
  curl -fsSL "$GOOD/install-app.sh" | LINGGEN_RELEASE_BASE="$GOOD" bash -s -- --no-launch >"$OUT/install-app.log" 2>&1
  check "app: install-app.sh over ${PREV_APP:-nothing}" $? "$(tail -3 "$OUT/install-app.log")"
  [ "$(app_version)" = "$APP_VERSION" ]; check "app: Linggen.app is the release" $? "got $(app_version)"
  launchctl setenv LINGGEN_RELEASE_BASE "$GOOD"
  open /Applications/Linggen.app
  engine_health 120; check "app: launches on the old home" $? "no 9527 health 120 s after open"
  sleep 5; quit_app
  launchctl unsetenv LINGGEN_RELEASE_BASE
  gap "app: in-app updater" "asks through a native confirm dialog (shell/src/main.rs) — not drivable headless"
fi

tcc_watch_stop "upgrade"

# ── Plugins: the old published plugin → this release's bundle ──────────────
if command -v claude >/dev/null; then
  { claude plugin marketplace remove linggen-memory && claude plugin marketplace add "$OUT/marketplace" \
      && claude plugin update linggen@linggen-memory; } >"$OUT/claude-plugin.log" 2>&1 \
    || claude plugin install linggen@linggen-memory >>"$OUT/claude-plugin.log" 2>&1
  check "claude: plugin updates" $? "$(tail -3 "$OUT/claude-plugin.log")"
  hooks="$(ls -td "$HOME"/.claude/plugins/cache/linggen-memory/linggen/*/hooks 2>/dev/null | head -1)"
  [ -n "$hooks" ] && diff -rq "$hooks" "$OUT/marketplace/plugins/linggen/hooks" >/dev/null
  check "claude: plugin cache carries the new hooks" $? "cache hooks '${hooks:-none}' differ from the release's"
else
  gap "claude: plugin updates" "no claude CLI in linggen-prev"
fi
exit 0
