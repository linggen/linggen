# Release gate — upgrade an old user (a clone of linggen-prev) to the release
# in the mirror, then the refusal and rollback cases (doc/test-design.md § 4).
# Env from the host: MIRROR, MODEL, DREAM_SHA (the release's built-in dream
# mission), and expect.env.
. ./common.sh
. ./expect.env

LING="$(ling_bin)"; MEM="$(mem_bin)"

# A swap restarted the engine: a new process on 9527 answering health, at the
# version the swapped-in binary carries.
engine_restarted() { # label old-pid want-version [log]
  local new v
  engine_health 60
  new="$(listener_pid 9527)"
  v="$(curl -fsS -m 3 http://127.0.0.1:9527/api/health 2>/dev/null | jq -r '.version // .data.version // empty')"
  if [ -n "$2" ] && [ -n "$new" ] && [ "$new" != "$2" ] && [ "$v" = "$3" ]; then
    pass "$1" "pid $2 → $new, v$v"
  else
    fail "$1" "pid ${2:-none} → ${new:-none}, 9527 answers v${v:-none} (want v$3)${4:+ — $(tail -2 "$4" 2>/dev/null)}"
  fi
}
PREV_LING="$(ver_of "$LING")"; PREV_MEM="$(ver_of "$MEM")"; PREV_APP="$(app_version)"
# The old binaries, for the rollback checks: install.sh keeps no .prev and a
# relabel swap keeps the release's own bytes, so a swap to it proves nothing.
cp "$LING" "$OUT/ling.old"; cp "$MEM" "$OUT/ling-mem.old"

# Make <bin>.prev an older binary than <bin>: the update's own, else the
# pre-upgrade copy. 1 = nothing older to roll back to (GAP).
real_prev() { # label bin old-copy
  local how="kept by the update"
  if [ ! -f "$2.prev" ] || [ "$(sha "$2.prev")" = "$(sha "$2")" ]; then
    if [ ! -f "$3" ] || [ "$(sha "$3")" = "$(sha "$2")" ]; then
      gap "$1: rollback reaches an older binary" "$(basename "$2").prev and the pre-upgrade binary are both the release's bytes"
      return 1
    fi
    how="the pre-upgrade binary, put there by the gate; the update kept $([ -f "$2.prev" ] && echo "the release's own bytes" || echo none)"
    rm -f "$2.prev"; cp "$3" "$2.prev"
  fi
  if [ "$(ver_of "$2.prev")" = "$(ver_of "$2")" ]; then
    gap "$1: rollback reaches an older version" "$(basename "$2").prev is v$(ver_of "$2") too ($how) — the swap is checked by bytes only"
  else
    info "$1: $(basename "$2").prev" "v$(ver_of "$2.prev"), $how"
  fi
}

# The rolled-back binary may predate --rollback: then the gate swaps back.
swap_back() { # bin
  mv "$1" "$1.gate" && mv "$1.prev" "$1" && mv "$1.gate" "$1.prev"
}
info "upgrade: from" "ling $PREV_LING, ling-mem $PREV_MEM, app ${PREV_APP:-none}"

# The old user, running: engine (fixtures config → the fake model) + ling-mem.
start_engine 90 || fail "upgrade: old engine runs" "no health"
mem_health 30 || start_mem 60
rows_before="$("$MEM" stats 2>/dev/null | jq -r '.total // empty')"
sessions_before="$(ls "$LG/sessions" 2>/dev/null | wc -l | tr -d ' ')"
info "upgrade: old state" "$rows_before memory rows, $sessions_before sessions"

tcc_watch_start

# ── Engine ─────────────────────────────────────────────────────────────────
engine_pid="$(listener_pid 9527)"; mem_pid="$(listener_pid 9528)"
OLD_RESTARTS=false
"$LING" update --help 2>/dev/null | grep -qi 'restart' && OLD_RESTARTS=true
if "$LING" update --help 2>/dev/null | grep -q -- '--rollback'; then
  LINGGEN_RELEASE_BASE="$GOOD" "$LING" update >"$OUT/ling-update.log" 2>&1
  check "ling update (from $PREV_LING)" $? "$(tail -3 "$OUT/ling-update.log")"
  VIA_INSTALL_SH=0
else
  gap "ling update (from $PREV_LING)" "the old binary predates LINGGEN_RELEASE_BASE — upgraded by re-running install.sh instead"
  curl -fsSL "$GOOD/install.sh" | LINGGEN_RELEASE_BASE="$GOOD" bash >"$OUT/install.log" 2>&1
  check "install.sh over a running install" $? "$(tail -3 "$OUT/install.log")"
  VIA_INSTALL_SH=1
fi
check_installed "upgrade: ling is the release" "$LING" "$LING_VERSION" "$LING_SHA"

# The engine that was serving must now run the new binary: a new process on
# 9527, started after the swap, answering health. The updater that ran is the
# OLD binary; one from before `ling update` restarted the engine says so in
# its --help — the release's own `ling update` is checked under § Restart.
if [ "$(sha "$OUT/ling.old")" = "$LING_SHA" ]; then
  gap "upgrade: 9527 restarted on the new ling" "the baseline already runs these bytes (v$PREV_LING) — nothing to swap"
elif [ "$VIA_INSTALL_SH" = 1 ] || $OLD_RESTARTS; then
  engine_restarted "upgrade: 9527 restarted on the new ling" "$engine_pid" "$LING_VERSION"
else
  gap "upgrade: 9527 restarted on the new ling" "ling $PREV_LING's \`ling update\` predates the engine restart — the release's own is checked under § Restart"
fi

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
v="$(mem_version)"; new_pid="$(listener_pid 9528)"
if [ "$(sha "$OUT/ling-mem.old")" = "$MEM_SHA" ]; then
  gap "upgrade: 9528 restarted on the new ling-mem" "the baseline already runs these bytes (v$PREV_MEM) — nothing to swap"
else
  [ "$v" = "$MEM_VERSION" ] && [ -n "$new_pid" ] && [ "$new_pid" != "$mem_pid" ]
  check "upgrade: 9528 restarted on the new ling-mem" $? \
    "9528 answers v${v:-none} (pid ${mem_pid:-none} → ${new_pid:-none}) after the binary swap — restarting it by hand" \
    "v$PREV_MEM → v$v, pid $mem_pid → $new_pid"
fi
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

# ── Rollback (the engine running: the release's swap restarts it) ─────────
if [ ! -f "$LING.prev" ]; then # install.sh path: give `ling update` one real swap first
  LINGGEN_RELEASE_BASE="$RELABEL" "$LING" update >"$OUT/ling-update-relabel.log" 2>&1
  check "ling update: installs and keeps ling.prev" $? "$(tail -2 "$OUT/ling-update-relabel.log")"
fi
if real_prev "ling" "$LING" "$OUT/ling.old"; then
  start_engine 90 || fail "rollback: engine runs before the swap" "no health"
  engine_pid="$(listener_pid 9527)"
  a="$(sha "$LING")"; p="$(sha "$LING.prev")"; pv="$(ver_of "$LING.prev")"; prev_restarts=false
  "$LING.prev" update --help 2>/dev/null | grep -qi 'restart' && prev_restarts=true
  "$LING" update --rollback >"$OUT/ling-rollback.log" 2>&1
  rc=$?
  [ "$rc" = 0 ] && [ "$(sha "$LING")" = "$p" ] && [ "$(sha "$LING.prev")" = "$a" ] && [ "$(ver_of "$LING")" = "$pv" ]
  check "ling update --rollback swaps to ling.prev" $? "exit $rc, now v$(ver_of "$LING") (want v$pv) — $(tail -2 "$OUT/ling-rollback.log")" "v$LING_VERSION → v$pv"
  engine_restarted "ling update --rollback: 9527 restarted on ling.prev" "$engine_pid" "$pv" "$OUT/ling-rollback.log"
  engine_pid="$(listener_pid 9527)"
  if "$LING" update --help 2>/dev/null | grep -q -- '--rollback'; then
    "$LING" update --rollback >>"$OUT/ling-rollback.log" 2>&1
    [ "$(sha "$LING")" = "$a" ]; check "ling update --rollback again returns" $? "sha ${a:0:12} not back"
    if $prev_restarts; then
      engine_restarted "ling update --rollback again: 9527 back on the release" "$engine_pid" "$LING_VERSION" "$OUT/ling-rollback.log"
    else
      info "ling update --rollback again: 9527 back on the release" "ling $pv's rollback predates the engine restart"
    fi
  else
    gap "ling update --rollback again returns" "ling $pv has no --rollback — the gate swaps the release back"
    stop_engine; swap_back "$LING"
  fi
fi

# ── Restart: the release's own `ling update` over a running engine ────────
stop_engine; start_engine 90 || fail "restart: engine runs before the update" "no health"
engine_pid="$(listener_pid 9527)"
LINGGEN_RELEASE_BASE="$RELABEL" "$LING" update >"$OUT/ling-update-restart.log" 2>&1
check "restart: ling update (the release, to ${LING_VERSION}-gate)" $? "$(tail -3 "$OUT/ling-update-restart.log")"
engine_restarted "restart: ling update restarts 9527 on the new ling" "$engine_pid" "$LING_VERSION" "$OUT/ling-update-restart.log"
stop_engine

if [ ! -f "$MEM.prev" ]; then
  mem_pid="$(listener_pid 9528)"
  LINGGEN_RELEASE_BASE="$GOOD" "$MEM" upgrade --yes --force >"$OUT/mem-upgrade-force.log" 2>&1
  check "ling-mem upgrade --force keeps ling-mem.prev" $? "$(tail -2 "$OUT/mem-upgrade-force.log")"
  new_pid="$(listener_pid 9528)"; v="$(mem_version)"
  [ -n "$mem_pid" ] && [ -n "$new_pid" ] && [ "$new_pid" != "$mem_pid" ] && [ "$v" = "$MEM_VERSION" ]
  check "ling-mem upgrade: 9528 restarted on the new binary" $? "pid ${mem_pid:-none} → ${new_pid:-none}, v${v:-none}" "pid $mem_pid → $new_pid, v$v"
fi
if real_prev "ling-mem" "$MEM" "$OUT/ling-mem.old"; then
  a="$(sha "$MEM")"; p="$(sha "$MEM.prev")"; pv="$(ver_of "$MEM.prev")"
  "$MEM" upgrade --rollback >"$OUT/mem-rollback.log" 2>&1
  rc=$?
  [ "$rc" = 0 ] && [ "$(sha "$MEM")" = "$p" ] && [ "$(ver_of "$MEM")" = "$pv" ] && mem_health 30
  check "ling-mem upgrade --rollback swaps to ling-mem.prev" $? "exit $rc, now v$(ver_of "$MEM") (want v$pv) — $(tail -2 "$OUT/mem-rollback.log")" "v$MEM_VERSION → v$pv"
  if "$MEM" upgrade --help 2>/dev/null | grep -q -- '--rollback'; then
    "$MEM" upgrade --rollback >>"$OUT/mem-rollback.log" 2>&1
    [ "$(sha "$MEM")" = "$a" ] && mem_health 30; check "ling-mem upgrade --rollback again returns" $? "sha ${a:0:12} not back"
  else
    gap "ling-mem upgrade --rollback again returns" "ling-mem $pv has no --rollback — the gate swaps the release back"
    stop_mem; swap_back "$MEM"; start_mem 60
  fi
fi

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

# ── Plugins: the old plugin → this release's bundle, via install-plugin.sh ─
if command -v claude >/dev/null; then
  clt_dialog_close
  install_plugin "$OUT/install-plugin.log"
  check_plugins "upgrade" $? "$OUT/install-plugin.log"
else
  gap "claude: plugin updates" "no claude CLI in linggen-prev"
fi
exit 0
