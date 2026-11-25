# Release gate — upgrade an old user (a clone of linggen-prev) to the release
# in the mirror, then the refusal and rollback cases (doc/test-design.md § 4).
# Env from the host: MIRROR, MODEL, BUILTIN_BODIES (each built-in mission's
# name=sha of its instructions), and expect.env. Memory is inside the engine: no ling-mem upgrade.
# What the baseline runs decides the memory checks: an old ling-mem daemon on
# the old port (≤ 2.2.x) is taken over (takeover.sh); a `ling` that already
# hosts memory (2.3.0 on) has its store opened in place (inplace.sh).
. ./common.sh
. ./expect.env
. ./takeover.sh
. ./inplace.sh
. ./notes.sh

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
# The old binary, for the rollback checks: install.sh keeps no .prev and a
# relabel swap keeps the release's own bytes, so a swap to it proves nothing.
cp "$LING" "$OUT/ling.old"

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
# A baseline that hosts memory has the stand-in and nothing on 9528 to wait for.
is_standin "$MEM" || mem_health 30 || start_mem 60
if [ -n "$(listener_pid "$LP")" ]; then MEM_KIND=takeover; takeover_seed
elif engine_mem_health 30; then MEM_KIND=inplace; inplace_seed
else
  MEM_KIND=none
  fail "upgrade: the baseline's memory answers" "nothing listens on $LP and $ENGINE_MEM/api/health is not ok"
fi
# The baseline's own record of the move to notes, if it ever ran 3.0: an old
# user has none; a VM kept as the next baseline (--save-prev) does.
NOTES_RECORD_BEFORE=0; [ -e "$NOTES_MOVED" ] && NOTES_RECORD_BEFORE=1
rows_before="$("$MEM" stats 2>/dev/null | jq -r '.total // empty')"
sessions_before="$(ls "$LG/sessions" 2>/dev/null | wc -l | tr -d ' ')"
info "upgrade: old state" "$rows_before memory rows, $sessions_before sessions"

tcc_watch_start

# ── Engine ─────────────────────────────────────────────────────────────────
engine_pid="$(listener_pid 9527)"
OLD_RESTARTS=false
"$LING" update --help 2>/dev/null | grep -qi 'restart' && OLD_RESTARTS=true
# A local build can carry the baseline's own version number (the version is
# stamped at release); the updater skips an equal version, so it is offered
# the relabelled manifest ("<v>-gate") — the same bytes.
UPDATE_BASE="$GOOD"
if [ "$PREV_LING" = "$LING_VERSION" ]; then
  UPDATE_BASE="$RELABEL"
  info "upgrade: same version number" "the baseline is v$PREV_LING too — updating from the relabelled manifest (${LING_VERSION}-gate, same bytes)"
fi
if "$LING" update --help 2>/dev/null | grep -q -- '--rollback'; then
  LINGGEN_RELEASE_BASE="$UPDATE_BASE" "$LING" update >"$OUT/ling-update.log" 2>&1
  rc=$?
  VIA_INSTALL_SH=0
  # Updaters before 1.10.0 gave a new binary 10 s to answer --version, and a
  # new file's first run waits on the macOS scan (8-22 s in this VM). They
  # restore the old binary; a user re-runs install.sh, so the gate does too.
  if [ $rc != 0 ] && grep -q 'no answer within 10s' "$OUT/ling-update.log"; then
    gap "ling update (from $PREV_LING)" "the old updater's 10 s start limit lost to the macOS scan — it restored $PREV_LING; upgraded by re-running install.sh"
    curl -fsSL "$GOOD/install.sh" | LINGGEN_RELEASE_BASE="$GOOD" bash >"$OUT/install.log" 2>&1
    check "install.sh after the old updater gave up" $? "$(tail -3 "$OUT/install.log")"
    VIA_INSTALL_SH=1
  else
    check "ling update (from $PREV_LING)" $rc "$(tail -3 "$OUT/ling-update.log")"
  fi
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

# ── Memory: taken over by the engine, or opened in place ──────────────────
if [ "$TK_SEEDED" = 1 ]; then
  # An old updater that does not restart the engine leaves the old one serving.
  engine_mem_health 5 || { stop_engine; start_engine 90; }
  takeover_after upgrade "$((TK_BACKUPS_BEFORE + 1))"
  TK_BACKUPS_AFTER="$((TK_BACKUPS_BEFORE + 1))"
elif [ "$IP_SEEDED" = 1 ]; then
  [ "$(ip_engine_version)" = "$LING_VERSION" ] || { stop_engine; start_engine 90; }
  inplace_after
  gap "takeover" "the baseline's ling v$PREV_LING already hosts memory — no old ling-mem daemon to take over; the in-place checks ran instead"
elif [ "$MEM_KIND" = inplace ]; then
  gap "in place" "the rows were not written through the baseline engine — the in-place checks did not run"
else
  gap "takeover" "no old ling-mem daemon served the baseline (a baseline already on 2.3?) — the takeover checks did not run"
fi

# ── The upgraded engine on the old home ────────────────────────────────────
stop_engine
start_engine 90
check "upgrade: new engine starts on the old home" $? "no health — $(tail -3 "$LG/ling.log" 2>/dev/null)"
engine_mem_health 60
check "upgrade: memory answers through the engine after a restart" $? "no ok from $ENGINE_MEM/api/health"
nothing_on "$LP"
check "upgrade: nothing listens on the old port $LP" $? "$PORT_DETAIL" "$PORT_DETAIL"

# Memory V2 is opt-in in 2.7: an upgrade with no `[memory]` written as v2
# stays on v1 and moves nothing (notes.sh). The move itself, written as v2, is
# rehearsed on the host (notes-local.sh). The VM is not switched to v2 here: a
# green run is kept as the next baseline, which must still be an un-moved v1
# home. A baseline that already holds the record or notes is a GAP.
if [ "$NOTES_RECORD_BEFORE" = 1 ]; then
  gap "upgrade: notes: an upgrade on the default stays v1, nothing moved" "the baseline already holds .state/moved-from-v1.json (it ran v2 before)"
else
  notes_stays_v1 "upgrade: notes (the default)"
fi

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
# Built-in missions refresh on startup (extensions/missions/builtin.rs): for
# each one the release ships, the installed instructions — everything after
# the front matter — are the release's, every memory tool they name is one
# the engine's memory server lists, and the only MCP server they allow is
# the engine's own memory server.
[ -n "${BUILTIN_BODIES:-}" ]
check "upgrade: the host named the release's built-in missions" $? "BUILTIN_BODIES unset" "$(tr ',' '\n' <<<"${BUILTIN_BODIES:-}" | cut -d= -f1 | tr '\n' ' ')"
# A mission may name a memory tool of the engine's own memory server, or one
# the one door lists (the chronicle mission's `memory_notes`).
mcp_answers "$ENGINE_MEM/mcp"; INTERNAL_NAMES="$MCP_NAMES"
mcp_answers "http://127.0.0.1:$EP/mcp"
MCP_NAMES="$INTERNAL_NAMES"$'\n'"$MCP_NAMES"
IFS=',' read -r -a builtins <<<"${BUILTIN_BODIES:-}"
for kv in "${builtins[@]}"; do
  name="${kv%%=*}"; want="${kv#*=}"; file="$LG/missions/$name/mission.md"
  body="$(awk 'f; /^---$/ {c++; if (c == 2) f = 1}' "$file" 2>/dev/null | shasum -a 256 | awk '{print $1}')"
  [ -f "$file" ] && [ "$body" = "$want" ]
  check "upgrade: built-in mission $name — its instructions are this release's" $? "installed ${body:0:12} ($( [ -f "$file" ] && echo present || echo missing)), release ${want:0:12}"
  missing=""
  for n in $(grep -oE 'memory_[a-z_]+' "$file" 2>/dev/null | sort -u); do
    grep -qx "$n" <<<"$MCP_NAMES" || missing="$missing $n"
  done
  for s in $(grep -oE 'mcp__[a-z0-9-]+' "$file" 2>/dev/null | sort -u); do
    [ "$s" = mcp__memory ] || missing="$missing $s"
  done
  [ -f "$file" ] && [ -n "$MCP_NAMES" ] && [ -z "$missing" ]
  check "upgrade: built-in mission $name — names only tools the engine serves" $? "not served:${missing:- none}; served: $(tr '\n' ' ' <<<"$MCP_NAMES" | cut -c1-200)"
done

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

# ── Takeover: writes after it, a rollback to both old programs, again ─────
if [ "$TK_SEEDED" = 1 ]; then
  takeover_writes
  takeover_rollback "$OUT/ling.old"
  takeover_again
fi
[ "$IP_SEEDED" = 1 ] && inplace_writes
stop_engine

# ── Refusal: every sha256 in the mirror wrong ──────────────────────────────
before="$(sha "$LING")"
LINGGEN_RELEASE_BASE="$BAD" "$LING" update >"$OUT/ling-update-bad.log" 2>&1
rc=$?
[ "$rc" != 0 ] && [ "$(sha "$LING")" = "$before" ] && "$LING" --version >/dev/null 2>&1
check "ling update: a wrong sha256 is refused, old binary runs" $? "exit $rc — $(tail -2 "$OUT/ling-update-bad.log")" "$(grep -m1 -i 'integrity\|sha' "$OUT/ling-update-bad.log")"

info "ling-mem upgrade: refusal and rollback" "not run — ling-mem is the engine's stand-in, with no updater of its own"

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
  prev_hosts_memory=false; "$LING.prev" mem --help >/dev/null 2>&1 && prev_hosts_memory=true
  "$LING" update --rollback >"$OUT/ling-rollback.log" 2>&1
  rc=$?
  [ "$rc" = 0 ] && [ "$(sha "$LING")" = "$p" ] && [ "$(sha "$LING.prev")" = "$a" ] && [ "$(ver_of "$LING")" = "$pv" ]
  check "ling update --rollback swaps to ling.prev" $? "exit $rc, now v$(ver_of "$LING") (want v$pv) — $(tail -2 "$OUT/ling-rollback.log")" "v$LING_VERSION → v$pv"
  engine_restarted "ling update --rollback: 9527 restarted on ling.prev" "$engine_pid" "$pv" "$OUT/ling-rollback.log"
  engine_pid="$(listener_pid 9527)"
  # An engine from before 2.3.0 starts its memory as `ling-mem start`, and
  # ~/.local/bin/ling-mem is the stand-in, which starts nothing: the rollback
  # puts ling-mem.prev back before that engine starts.
  if [ "$TK_SEEDED" = 1 ]; then
    takeover_engine_rollback "$pv" "$prev_hosts_memory"
  elif [ "$IP_SEEDED" = 1 ] && $prev_hosts_memory; then
    inplace_rollback "$pv"
  elif [ "$IP_SEEDED" = 1 ]; then
    gap "ling update --rollback: memory on ling.prev" "ling.prev (v$pv) does not host memory — not the in-place baseline's engine"
  else
    gap "ling update --rollback: memory on ling.prev" "the baseline had no old ling-mem daemon, so there is no memory to find on the rolled-back engine"
  fi
  if "$LING" update --help 2>/dev/null | grep -q -- '--rollback'; then
    "$LING" update --rollback >>"$OUT/ling-rollback.log" 2>&1
    [ "$(sha "$LING")" = "$a" ]; check "ling update --rollback again returns" $? "sha ${a:0:12} not back"
    if $prev_restarts; then
      if [ "$TK_SEEDED" = 1 ]; then takeover_engine_forward
      elif [ "$IP_SEEDED" = 1 ] && $prev_hosts_memory; then inplace_forward
      else engine_mem_health 60; check "ling update --rollback again: memory answers through the engine" $? "no ok from $ENGINE_MEM/api/health"; fi
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

# ── App ────────────────────────────────────────────────────────────────────
# What the in-app updater leaves behind: the bundle swapped, the OLD engine
# still running and healthy. The new shell must replace it (shell/src/main.rs
# upgrade_engine): asked to restart first, reclaimed if that does not take.
# The updater itself is a native dialog (a GAP below), so the state is made
# here the way update-helper.sh makes it: old bundle moved aside, new one in.
OLD_APP=/tmp/gate-old-app/Linggen.app
engine_version() { curl -fsS -m 3 http://127.0.0.1:9527/api/health 2>/dev/null | jq -r '.version // empty'; }
older_than() { [ "$1" != "$2" ] && [ "$(printf '%s\n%s\n' "$1" "$2" | sort -V | head -1)" = "$1" ]; }
wait_engine_version() { # want seconds
  local i
  for ((i = 0; i < $2; i += 3)); do [ "$(engine_version)" = "$1" ] && { UPGRADED_AFTER=$i; return 0; }; sleep 3; done
  return 1
}
# Bundled skills (shell/src/skill_sync.rs): the new app replaces an older
# installed skill's code with its bundle's and never touches the skill's state.
# linggen-prev installs the old app with --no-launch, so 灵境 is laid down here
# the way a 2.0.x shell did on first open — a plain copy of its bundle, no
# marker. Then, before install-app.sh: a state file under data/, and a comment
# appended to state.mjs, so the check proves a replace even when the old
# bundle already carried the release's state.mjs.
SKILL_CODE=scripts/state.mjs
SKILL_PLANT=data/gate-planted-state.json
APP_SKILLS=Contents/Resources/app-resources
skill_state_plant() {
  local g="$LG/skills/lingjing" old="$OLD_APP/$APP_SKILLS/skills/lingjing"
  PLANT_SHA=""
  if [ ! -f "$g/$SKILL_CODE" ] && [ -f "$old/$SKILL_CODE" ]; then
    mkdir -p "$LG/skills" && ditto "$old" "$g"
    info "skills: baseline 灵境" "copied from the old app's bundle (what its shell did on first open)"
  fi
  [ -f "$g/$SKILL_CODE" ] || { info "skills: baseline 灵境" "none — neither installed nor in the old app's bundle"; return; }
  mkdir -p "$g/data"
  printf '{"gate":"%s","save":"kept byte for byte"}\n' "$(date +%s)-$RANDOM" >"$g/$SKILL_PLANT"
  PLANT_SHA="$(sha "$g/$SKILL_PLANT")"
  printf '\n// gate: the baseline copy, changed so only a replace makes it the release'"'"'s\n' >>"$g/$SKILL_CODE"
  info "skills: baseline 灵境" "$SKILL_CODE $(sha "$g/$SKILL_CODE" | cut -c1-12), marker: $(jq -c '{ref,by,committed}' "$g/.installed" 2>/dev/null || echo none)"
}
skill_update_check() {
  local g="$LG/skills/lingjing" res="/Applications/Linggen.app/$APP_SKILLS"
  local name="app: an installed 灵境 gets the bundle's code" have want
  [ -f "$res/skills-stamp.json" ]
  check "app: the bundle carries skills-stamp.json" $? "missing — the shell falls back to adding missing files only, and no installed skill is updated" "$(jq -c . "$res/skills-stamp.json" 2>/dev/null)"
  if [ ! -f "$res/skills/lingjing/$SKILL_CODE" ]; then gap "$name" "the release bundle has no lingjing/$SKILL_CODE"; return; fi
  if [ -z "$PLANT_SHA" ]; then
    gap "$name" "no older 灵境 on the baseline — the release's sync was a fresh install"
    gap "app: 灵境 data/ kept across the app update" "no baseline 灵境 to plant state in"
    return
  fi
  have="$(sha "$g/$SKILL_CODE")"; want="$(sha "$res/skills/lingjing/$SKILL_CODE")"
  [ -n "$have" ] && [ "$have" = "$want" ]
  check "$name" $? "installed ${have:-none} ≠ bundle $want — marker: $(head -c 160 "$g/.installed" 2>/dev/null)" "${want:0:12}, marker $(jq -c '{ref,by}' "$g/.installed" 2>/dev/null)"
  [ "$(sha "$g/$SKILL_PLANT")" = "$PLANT_SHA" ]
  check "app: 灵境 data/ kept across the app update" $? "$SKILL_PLANT is $(sha "$g/$SKILL_PLANT" || echo gone), was $PLANT_SHA"
}

app_engine_upgrade_check() {
  local name="app: an older engine left running is replaced by the new app" new=/tmp/gate-new-app/Linggen.app old_v new_v
  if [ ! -d "$OLD_APP" ]; then gap "$name" "no older Linggen.app on the baseline"; return; fi
  quit_app; stop_engine
  new_v="$(ver_of /Applications/Linggen.app/Contents/MacOS/ling)"
  old_v="$(ver_of "$OLD_APP/Contents/MacOS/ling")"
  if ! older_than "$old_v" "$new_v"; then gap "$name" "the baseline app's engine ($old_v) is not older than the release's ($new_v)"; return; fi
  rm -rf /tmp/gate-new-app; mkdir -p /tmp/gate-new-app
  mv /Applications/Linggen.app "$new" && mv "$OLD_APP" /Applications/Linggen.app
  (cd "$HOME" && nohup /Applications/Linggen.app/Contents/MacOS/ling --web --idle-shutdown-secs 300 >"$OUT/old-engine.log" 2>&1 &)
  engine_health 90 && [ "$(engine_version)" = "$old_v" ]
  if [ $? != 0 ]; then
    fail "$name" "the old engine ($old_v) did not come up: health says '$(engine_version)'"
    pkill -f '/ling --web' 2>/dev/null; rm -rf /Applications/Linggen.app; mv "$new" /Applications/Linggen.app; return
  fi
  mv /Applications/Linggen.app /tmp/gate-old-app/Prev.app && mv "$new" /Applications/Linggen.app && rm -rf /tmp/gate-old-app/Prev.app
  open_app /Applications/Linggen.app
  wait_engine_version "$new_v" 300
  check "$name" $? "9527 still answers '$(engine_version)' 300 s after open (want $new_v, was $old_v)" "$old_v → $new_v after ${UPGRADED_AFTER:-?} s"
  pgrep -f '/Linggen.app/Contents/MacOS/linggen-shell' >/dev/null
  check "app: still running after replacing the engine" $? "no Linggen.app shell process"
  quit_app
}

if [ -n "$APP_VERSION" ]; then
  rm -rf /tmp/gate-old-app; mkdir -p /tmp/gate-old-app
  [ -d /Applications/Linggen.app ] && ditto /Applications/Linggen.app "$OLD_APP"
  skill_state_plant
  curl -fsSL "$GOOD/install-app.sh" | LINGGEN_RELEASE_BASE="$GOOD" bash -s -- --no-launch >"$OUT/install-app.log" 2>&1
  check "app: install-app.sh over ${PREV_APP:-nothing}" $? "$(tail -3 "$OUT/install-app.log")"
  [ "$(app_version)" = "$APP_VERSION" ]; check "app: Linggen.app is the release" $? "got $(app_version)"
  launchctl setenv LINGGEN_RELEASE_BASE "$GOOD"
  t_open=$SECONDS
  open_app /Applications/Linggen.app
  engine_health 120; check "app: launches on the old home" $? "no 9527 health 120 s after open"
  for _ in $(seq 20); do [ "$(jq -r '.by // empty' "$LG/skills/lingjing/.installed" 2>/dev/null)" = app ] && break; sleep 3; done
  check_assets_heal "app (upgrade)" 600 upgrade "$t_open"
  sleep 5; quit_app
  skill_update_check
  app_engine_upgrade_check
  launchctl unsetenv LINGGEN_RELEASE_BASE
  gap "app: in-app updater" "asks through a native confirm dialog (shell/src/main.rs) — not drivable headless"
fi

tcc_watch_stop "upgrade"

# ── Plugins: linggen@linggen-memory → this release's linggen@linggen ──────
# install-plugin.sh retires the old plugin and installs the bundle. linggen-prev
# has no Codex; the host's CLI is handed in so the new plugin is checked there.
[ -x "$OUT/codex" ] && ! command -v codex >/dev/null && mkdir -p "$HOME/.local/bin" && cp "$OUT/codex" "$HOME/.local/bin/codex"
if command -v claude >/dev/null; then
  claude_plugin_block "$OLD_PLUGIN_ID" | grep -q enabled \
    && info "upgrade: before — claude has $OLD_PLUGIN_ID" "as the previous release installed it" \
    || gap "upgrade: before — claude has $OLD_PLUGIN_ID" "linggen-prev has no old plugin enabled — the retire step is not exercised"
  # The old plugin's memory server, before the new plugin replaces it: the
  # engine refuses it with "update the plugin", never serves it. The app was
  # quit above, so the release's engine is started as a person starts it.
  engine_health 5 || start_engine 90
  if engine_health 5; then
    mcp_retired "http://127.0.0.1:$EP/mcp/memory"
    check "upgrade: before — the old plugin's memory address answers 410 UPGRADE_PLUGIN" $? "$RETIRED_DETAIL" "$RETIRED_DETAIL"
    # The same call as Claude Code makes it (its User-Agent names it): the
    # engine notes the host, and `ling status` says how to update it.
    curl -sS -m 10 -o /dev/null -A 'claude-code/2.1 (cli)' -X POST -H 'content-type: application/json' \
      -d '{"jsonrpc":"2.0","id":1,"method":"tools/list","params":{}}' "http://127.0.0.1:$EP/mcp/memory" 2>/dev/null
    "$LING" status >"$OUT/status-old-plugin.txt" 2>&1
    grep -q 'Old plugin' "$OUT/status-old-plugin.txt" && grep -q 'Claude Code called the retired' "$OUT/status-old-plugin.txt"
    check "upgrade: ling status names Claude Code's old plugin and its update" $? "$(grep -i -A1 'old plugin' "$OUT/status-old-plugin.txt" | head -c 240)" "$(grep 'Claude Code called' "$OUT/status-old-plugin.txt" | head -c 160)"
  else
    fail "upgrade: before — the release's engine starts for the plugin step" "no health on $EP — $(tail -3 "$OUT/ling-start.log" 2>/dev/null)"
  fi
  clt_dialog_close
  install_plugin "$OUT/install-plugin.log"
  check_plugins "upgrade" $? "$OUT/install-plugin.log"
else
  gap "claude: plugin updates" "no claude CLI in linggen-prev"
fi
exit 0
