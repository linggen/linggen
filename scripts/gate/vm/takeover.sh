# Release gate — the engine takes memory over from an old ling-mem daemon
# (engine 2.3.0 on: memory_host/takeover.rs, standin.rs, legacy_port.rs).
# Sourced by upgrade.sh after common.sh; the old user's engine and its
# ling-mem daemon are running when takeover_seed is called.
#
#   takeover_seed            rows written through the old daemon, its pid, every id
#   takeover_after <label>   the named checks once the new engine runs
#   takeover_writes          a write through the stand-in and one through the engine
#   takeover_rollback        the previous engine + ling-mem.prev back: every row there
#   takeover_again           the release again: the old daemon stopped, no second backup
#   takeover_engine_rollback `ling update --rollback` to the previous release: its memory answers
#   takeover_engine_forward  `ling update --rollback` again: the engine holds the store, every row there
#
# Needs LING (the installed ling), OUT, and the release's bytes at $LING when
# takeover_rollback is called.

OLD_MEM="http://127.0.0.1:$LP"
TK_IDS="$OUT/takeover-ids.tsv"       # id<TAB>tier<TAB>written by
TK_ALL="$OUT/takeover-ids-all.txt"   # every id the old store held, then every id written since
STANDIN="$HOME/.local/bin/ling-mem"
: >"$TK_IDS"; : >"$TK_ALL"
# Set by takeover_seed; TK_SEEDED=0 (no old daemon to take over) skips the rest.
TK_SEEDED=0 TK_BACKUPS_BEFORE=0 TK_BACKUPS_AFTER=1 TK_OLD_MEM_SHA="" TK_CORE="" OLD_DAEMON_PID=""

tk_backups() { ls -d "$LG"/memory/backups/takeover-*/ 2>/dev/null | wc -l | tr -d ' '; }
tk_marker() { echo "$LG/memory/backups/takeover-$(ver_of "$LING").done"; }

# Every id in a store, both tables.
tk_ids() { # base
  mem_post "$1" /api/memory/list '{"limit":5000,"include_expired":true}' | jq -r '.data[]?.id' 2>/dev/null
}

tk_add() { # base by tier(core|semantic|staged) content → id appended to TK_IDS
  local body id
  body="$(jq -nc --arg c "$4" --arg t "$3" '{content: $c, type: "fact", global: true, skip_dedup: true}
    + (if $t == "staged" then {episodic: true} else {tier: $t} end)')"
  id="$(mem_post "$1" /api/memory/add "$body" | jq -r '.data.fact.id // .data.id // empty' 2>/dev/null)"
  [ -n "$id" ] || return 1
  printf '%s\t%s\t%s\n' "$id" "$3" "$2" >>"$TK_IDS"
  printf '%s\n' "$id" >>"$TK_ALL"
}

# Each id in a file found by id at base; TK_MISSING lists the ones not found.
tk_find_all() { # base file
  local id n=0
  TK_MISSING=""
  while read -r id; do
    [ -n "$id" ] || continue
    n=$((n + 1))
    mem_post "$1" /api/memory/get "$(jq -nc --arg i "$id" '{id: $i}')" | jq -e --arg i "$id" '.data.id == $i' >/dev/null 2>&1 \
      || TK_MISSING="$TK_MISSING $id"
  done <"$2"
  TK_FOUND="$n"
  [ -z "$TK_MISSING" ]
}

# The tier each seeded row was written at, read back (staged = episodic).
tk_tiers_kept() { # base
  local id tier by got want bad=""
  while IFS=$'\t' read -r id tier by; do
    [ "$by" = "old daemon" ] || continue
    want="$tier"; [ "$tier" = staged ] && want=episodic
    got="$(mem_post "$1" /api/memory/get "$(jq -nc --arg i "$id" '{id: $i}')" | jq -r '.data.tier // empty' 2>/dev/null)"
    [ "$got" = "$want" ] || bad="$bad $id:$want→${got:-gone}"
  done <"$TK_IDS"
  TK_TIER_DETAIL="$bad"
  [ -z "$bad" ]
}

TK_STAMP="gate-$(date +%s)"
takeover_seed() {
  local i ok=0
  TK_BACKUPS_BEFORE="$(tk_backups)"
  TK_OLD_MEM_SHA="$(sha "$STANDIN")"
  OLD_DAEMON_PID="$(listener_pid "$LP")"
  local comm; comm="$(ps -o comm= -p "${OLD_DAEMON_PID:-0}" 2>/dev/null)"
  case "$comm" in
    *ling-mem*) pass "takeover: before — the old ling-mem daemon serves the store" "pid $OLD_DAEMON_PID ($comm), v$(mem_version)" ;;
    *) fail "takeover: before — the old ling-mem daemon serves the store" "9528 is pid ${OLD_DAEMON_PID:-none} (${comm:-nothing})"; return 1 ;;
  esac
  tk_ids "$OLD_MEM" >"$TK_ALL"
  # Three long-term rows, one staged, one core: written through the old daemon.
  for i in 1 2 3; do
    tk_add "$OLD_MEM" "old daemon" semantic "Gate takeover long-term row $i ($TK_STAMP): Alex keeps the harbor ledger in a green notebook." && ok=$((ok + 1))
  done
  tk_add "$OLD_MEM" "old daemon" staged "Gate takeover staged row ($TK_STAMP): Alex walked the pier at dawn." && ok=$((ok + 1))
  TK_CORE="Gate takeover core row ($TK_STAMP): Alex's middle name is Harbor."
  tk_add "$OLD_MEM" "old daemon" core "$TK_CORE" && ok=$((ok + 1))
  [ "$ok" = 5 ]
  check "takeover: before — 5 rows written through the old daemon (3 long-term, 1 staged, 1 core)" $? \
    "$ok of 5 written — $(tail -1 "$OUT/ling-mem.log" 2>/dev/null | head -c 160)" "$(wc -l <"$TK_ALL" | tr -d ' ') ids in the store"
  TK_SEEDED=1
}

# The named checks once the new engine serves: phase is "upgrade" or "again".
takeover_after() { # phase want-backups
  local p="takeover ($1)" epid lock_pid lock_port left marker n
  engine_mem_health 60
  check "$p: memory answers through the engine (/mem/api/health)" $? "no ok from $ENGINE_MEM/api/health — $(tail -2 "$LG/ling.log" 2>/dev/null)" "v$(curl -fsS -m 3 "$ENGINE_MEM/api/health" | jq -r '.data.version // empty')"
  epid="$(listener_pid "$EP")"

  left="$(ling_mem_procs)"
  ! kill -0 "${OLD_DAEMON_PID:-0}" 2>/dev/null && [ -z "$left" ]
  check "$p: the old ling-mem daemon is gone" $? "pid ${OLD_DAEMON_PID:-?} alive=$(kill -0 "${OLD_DAEMON_PID:-0}" 2>/dev/null && echo yes || echo no); ling-mem processes: ${left:-none}" "pid ${OLD_DAEMON_PID:-?} stopped"
  only_engine_on "$LP" "$epid"
  check "$p: only the engine listens on $LP, loopback only" $? "$PORT_DETAIL" "$PORT_DETAIL"

  n="$(tk_backups)"; marker="$(tk_marker)"
  [ "$n" = "$2" ] && [ -f "$marker" ]
  check "$p: $2 store backup(s) and the marker $(basename "$marker")" $? \
    "$n takeover-* backups (want $2), marker $([ -f "$marker" ] && echo present || echo missing): $(ls "$LG/memory/backups" 2>/dev/null | tr '\n' ' ' | head -c 160)" \
    "$(ls -d "$LG"/memory/backups/takeover-*/ 2>/dev/null | xargs -n1 basename | tr '\n' ' ')"

  lock_pid="$(jq -r '.pid // empty' "$LG/memory/.store.lock" 2>/dev/null)"
  lock_port="$(jq -r '.port // empty' "$LG/memory/.store.lock" 2>/dev/null)"
  [ -n "$epid" ] && [ "$lock_pid" = "$epid" ] && [ "$lock_port" = "$EP" ]
  check "$p: the store lock names the engine's pid" $? "lock says pid ${lock_pid:-?} port ${lock_port:-?}, engine is pid ${epid:-none} on $EP" "pid $lock_pid, port $lock_port"

  tk_find_all "$ENGINE_MEM" "$TK_ALL"
  check "$p: every row written before is found by id through the engine" $? "missing:$TK_MISSING" "$TK_FOUND ids"
  tk_tiers_kept "$ENGINE_MEM"
  check "$p: the seeded rows keep their tiers (core, long-term, staged)" $? "$TK_TIER_DETAIL"
  "$LING" mem session-start --format json >"$OUT/takeover-session-start-$1.json" 2>&1 && grep -qF "$TK_CORE" "$OUT/takeover-session-start-$1.json"
  check "$p: ling mem session-start loads the old core row" $? "$(head -c 200 "$OUT/takeover-session-start-$1.json")"

  check_standin "$p" "$(ver_of "$LING")" "$TK_OLD_MEM_SHA"

  mcp_answers "http://127.0.0.1:$LP/mcp"
  check "$p: old plugins' 127.0.0.1:$LP/mcp answers" $? "$MCP_DETAIL" "$MCP_DETAIL"
  mcp_answers "http://127.0.0.1:$EP/mcp/memory"
  check "$p: $EP/mcp/memory answers" $? "$MCP_DETAIL" "$MCP_DETAIL"
}

# Writes after the upgrade: one through the stand-in (old hooks call it), one through the engine.
takeover_writes() {
  local out id
  out="$("$STANDIN" add "Gate takeover row through the stand-in ($TK_STAMP): Alex prefers rope knots." --tier semantic 2>&1)"
  id="$(jq -r '.fact.id // .data.fact.id // .id // empty' <<<"$out" 2>/dev/null)"
  [ -n "$id" ] && mem_post "$ENGINE_MEM" /api/memory/get "$(jq -nc --arg i "$id" '{id: $i}')" | jq -e --arg i "$id" '.data.id == $i' >/dev/null 2>&1
  check "takeover: a write through the stand-in ~/.local/bin/ling-mem lands" $? "$(head -c 200 <<<"$out")" "$id"
  [ -n "$id" ] && { printf '%s\tsemantic\tstand-in\n' "$id" >>"$TK_IDS"; printf '%s\n' "$id" >>"$TK_ALL"; }
  tk_add "$ENGINE_MEM" "engine" semantic "Gate takeover row through the engine ($TK_STAMP): Alex mends nets on Sundays."
  check "takeover: a write through the engine (/mem/api/memory/add)" $? "no id back"
  awk -F'\t' '$3 != "old daemon" {print $1}' "$TK_IDS" >"$OUT/takeover-ids-after.txt"
}

# Swap a binary in by a fresh inode, never over the running file.
tk_put() { # src dest
  cp "$1" "$2.gate-new" && mv -f "$2.gate-new" "$2"
}

tk_stop_engine() {
  local pid; pid="$(listener_pid "$EP")"
  "$LING" stop >/dev/null 2>&1 || true
  [ -n "$pid" ] && for _ in $(seq 20); do kill -0 "$pid" 2>/dev/null || break; sleep 1; done
  [ -n "$pid" ] && kill "$pid" 2>/dev/null
  sleep 1
}

# The previous engine and ling-mem.prev back, started: the rows written
# after the upgrade must be there, read by the old daemon. Then one more
# row through the old daemon, for takeover_again to find.
takeover_rollback() { # previous-engine-binary
  cp "$LING" "$OUT/ling.release"
  tk_stop_engine
  cp "$STANDIN" "$OUT/ling-mem.standin"
  tk_put "$1" "$LING"
  mv -f "$STANDIN.prev" "$STANDIN"
  start_mem 90
  start_engine 90
  local v_ling v_mem
  v_ling="$(curl -fsS -m 3 "http://127.0.0.1:$EP/api/health" | jq -r '.version // .data.version // empty')"
  v_mem="$(mem_version)"
  [ "$v_ling" = "$(ver_of "$1")" ] && [ "$v_mem" = "$(ver_of "$STANDIN")" ] && ! is_standin "$STANDIN"
  check "takeover rollback: the previous engine and ling-mem.prev run" $? "engine v${v_ling:-none}, 9528 v${v_mem:-none} — $(tail -2 "$OUT/ling-mem.log" 2>/dev/null)" "engine v$v_ling, ling-mem v$v_mem"
  tk_find_all "$OLD_MEM" "$OUT/takeover-ids-after.txt"
  check "takeover rollback: the rows written after the upgrade are there" $? "missing:$TK_MISSING" "$TK_FOUND ids, read by ling-mem v$v_mem"
  tk_find_all "$OLD_MEM" "$TK_ALL"
  check "takeover rollback: every older row is there too" $? "missing:$TK_MISSING" "$TK_FOUND ids"
  tk_add "$OLD_MEM" "old daemon, rollback" semantic "Gate takeover row during the rollback ($TK_STAMP): Alex repainted the skiff."
  check "takeover rollback: the old daemon writes" $? "no id back"
}

# The release again over the running old daemon: taken over again, and the
# marker keeps the store from a second backup for the same version.
takeover_again() {
  OLD_DAEMON_PID="$(listener_pid "$LP")"
  TK_OLD_MEM_SHA="$(sha "$STANDIN")"
  tk_stop_engine
  [ -n "$OLD_DAEMON_PID" ] && kill -0 "$OLD_DAEMON_PID" 2>/dev/null \
    || info "takeover (again): the old daemon" "stopped with the previous engine — the takeover below finds none running"
  tk_put "$OUT/ling.release" "$LING"
  start_engine 90
  check "takeover (again): the release starts over the old daemon" $? "no health — $(tail -3 "$LG/ling.log" 2>/dev/null)"
  takeover_after again "$TK_BACKUPS_AFTER"
}

# `ling update --rollback` ran and the previous engine was restarted. An
# engine without memory starts it as `ling-mem start`, so the rollback must
# have put ling-mem.prev back over the stand-in: the old daemon answers on
# the legacy port and reads every row, including those written after the
# upgrade. An engine that hosts memory serves the same port itself.
takeover_engine_rollback() { # prev-engine-version prev-engine-hosts-memory(true|false)
  local p="ling update --rollback" v
  mem_health 90
  check "$p: memory answers on $LP" $? "nothing answers on $LP — v$1 starts memory as 'ling-mem start'; $(tail -2 "$OUT/ling-rollback.log" 2>/dev/null)" "v$(mem_version)"
  if [ "$2" = false ]; then
    v="$(mem_version)"
    ! is_standin "$STANDIN" && [ ! -e "$STANDIN.prev" ] && [ -n "$(ling_mem_procs)" ] && [ "$v" = "$(ver_of "$STANDIN")" ]
    check "$p: the old ling-mem is back and serves $LP" $? "stand-in=$(is_standin "$STANDIN" && echo yes || echo no), ling-mem.prev=$([ -e "$STANDIN.prev" ] && echo present || echo gone), daemons: $(ling_mem_procs), 9528 v$v" "ling-mem v$v: $(grep -m1 'ling-mem' "$OUT/ling-rollback.log")"
  fi
  tk_find_all "$OLD_MEM" "$OUT/takeover-ids-after.txt"
  check "$p: the rows written after the upgrade are there on $LP" $? "missing:$TK_MISSING" "$TK_FOUND ids"
  tk_find_all "$OLD_MEM" "$TK_ALL"
  check "$p: every older row is there too" $? "missing:$TK_MISSING" "$TK_FOUND ids"
}

# Rolled forward again: the engine that hosts memory stopped the old daemon,
# took the store over and put the stand-in back.
takeover_engine_forward() {
  local p="ling update --rollback again" left
  engine_mem_health 60
  check "$p: memory answers through the engine" $? "no ok from $ENGINE_MEM/api/health — $(tail -2 "$LG/ling.log" 2>/dev/null)"
  left="$(ling_mem_procs)"
  [ -z "$left" ] && is_standin "$STANDIN"
  check "$p: the old daemon is gone and ~/.local/bin/ling-mem is the stand-in" $? "ling-mem processes: ${left:-none}; stand-in=$(is_standin "$STANDIN" && echo yes || echo no)"
  tk_find_all "$ENGINE_MEM" "$TK_ALL"
  check "$p: every row is found through the engine" $? "missing:$TK_MISSING" "$TK_FOUND ids"
}
