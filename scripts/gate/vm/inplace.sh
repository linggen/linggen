# Release gate — a baseline whose `ling` already hosts memory (2.3.0 on):
# there is no old daemon to take over; the release opens the same store in
# place (memory_host/mod.rs) and copies it once for its version before the
# first open (`memory/backups/takeover-<stamp>/`, named by the marker
# `takeover-<version>.done`). Sourced by upgrade.sh after takeover.sh (its
# tk_* helpers and id files); the baseline engine is running when
# inplace_seed is called.
#
#   inplace_seed               rows written through the baseline engine's /mem; the store's inode, backups, ling-mem.prev
#   inplace_after              the release serves the same store: every row and tier, one backup for its version
#   inplace_writes             a row written through the release
#   inplace_rollback <version> `ling update --rollback` ran: ling.prev finds every row, the release's included
#   inplace_forward            `ling update --rollback` again: the release finds every row, no second backup

IP_SEEDED=0 IP_INODE="" IP_PREV_SHA="" IP_BY="baseline engine"
IP_BACKUPS="$OUT/inplace-backups.txt"   # takeover-*/ dirs, as of the last look
IP_MARKERS="$OUT/inplace-markers.txt"   # takeover-*.done, as of the last look
IP_RELEASE_IDS="$OUT/inplace-ids-release.txt"

ip_store_inode() { stat -f %i "$LG/memory/memory.lancedb" 2>/dev/null; }
ip_backup_dirs() { (cd "$LG/memory/backups" 2>/dev/null && ls -d takeover-*/ 2>/dev/null | tr -d / | sort); }
ip_markers() { (cd "$LG/memory/backups" 2>/dev/null && ls takeover-*.done 2>/dev/null | sort); }
ip_snapshot() { ip_backup_dirs >"$IP_BACKUPS"; ip_markers >"$IP_MARKERS"; }
# Backup dirs there now that were not at the last snapshot (a pruned old one
# does not count against a new one).
ip_new_backups() { comm -13 "$IP_BACKUPS" <(ip_backup_dirs) | tr '\n' ' ' | sed 's/ $//'; }
ip_engine_version() { curl -fsS -m 3 "http://127.0.0.1:$EP/api/health" 2>/dev/null | jq -r '.version // .data.version // empty'; }
# `ling update --rollback` may return before the swapped-in engine answers.
ip_wait_engine_version() { # want seconds
  local i
  for ((i = 0; i < $2; i++)); do [ "$(ip_engine_version)" = "$1" ] && return 0; sleep 1; done
  return 1
}
ip_mem_version() { curl -fsS -m 3 "$ENGINE_MEM/api/health" 2>/dev/null | jq -r '.data.version // empty'; }

# The store lock names the engine serving on EP.
ip_lock_check() { # label
  local epid lock_pid lock_port
  epid="$(listener_pid "$EP")"
  lock_pid="$(jq -r '.pid // empty' "$LG/memory/.store.lock" 2>/dev/null)"
  lock_port="$(jq -r '.port // empty' "$LG/memory/.store.lock" 2>/dev/null)"
  [ -n "$epid" ] && [ "$lock_pid" = "$epid" ] && [ "$lock_port" = "$EP" ]
  check "$1: the store lock names the engine's pid" $? "lock says pid ${lock_pid:-?} port ${lock_port:-?}, engine is pid ${epid:-none} on $EP" "pid $lock_pid, port $lock_port"
}

ip_inode_check() { # label
  local inode; inode="$(ip_store_inode)"
  [ -n "$inode" ] && [ "$inode" = "$IP_INODE" ]
  check "$1: the store is opened in place" $? "memory/memory.lancedb inode ${inode:-gone}, was $IP_INODE" "memory/memory.lancedb, inode $inode as before"
}

ip_no_daemon_check() { # label
  local left; left="$(ling_mem_procs)"
  nothing_on "$LP" && [ -z "$left" ]
  check "$1: no ling-mem daemon, nothing listens on $LP" $? "$PORT_DETAIL; ling-mem processes: ${left:-none}" "$PORT_DETAIL"
}

ip_rows_check() { # label base
  tk_find_all "$2" "$TK_ALL"
  check "$1: every row is found by id" $? "missing:$TK_MISSING" "$TK_FOUND ids"
  tk_tiers_kept "$2" "$IP_BY"
  check "$1: the seeded rows keep their tiers (core, long-term, staged)" $? "$TK_TIER_DETAIL"
}

# A start that must not back the store up again: no new takeover-*/ dir.
ip_no_new_backup_check() { # label why
  local new; new="$(ip_new_backups)"
  [ -z "$new" ]
  check "$1: no new store backup" $? "new: $new" "$2"
}

inplace_seed() {
  local i ok=0 v
  v="$(ip_engine_version)"
  pass "in place: before — ling v$v hosts memory ($EP/mem), no daemon on $LP" "memory v$(ip_mem_version); $(ling_mem_procs | wc -w | tr -d ' ') ling-mem processes"
  tk_ids "$ENGINE_MEM" >"$TK_ALL"
  for i in 1 2 3; do
    tk_add "$ENGINE_MEM" "$IP_BY" semantic "Gate in-place long-term row $i ($TK_STAMP): Alex keeps the harbor ledger in a green notebook." && ok=$((ok + 1))
  done
  tk_add "$ENGINE_MEM" "$IP_BY" staged "Gate in-place staged row ($TK_STAMP): Alex walked the pier at dawn." && ok=$((ok + 1))
  TK_CORE="Gate in-place core row ($TK_STAMP): Alex's middle name is Harbor."
  tk_add "$ENGINE_MEM" "$IP_BY" core "$TK_CORE" && ok=$((ok + 1))
  [ "$ok" = 5 ]
  check "in place: before — 5 rows written through v$v's engine (3 long-term, 1 staged, 1 core)" $? \
    "$ok of 5 written" "$(wc -l <"$TK_ALL" | tr -d ' ') ids in the store"
  IP_INODE="$(ip_store_inode)"
  IP_PREV_SHA="$(sha "$STANDIN.prev")"
  ip_snapshot
  info "in place: before — the store" "memory.lancedb inode ${IP_INODE:-none}; backups: $(tr '\n' ' ' <"$IP_BACKUPS"); markers: $(tr '\n' ' ' <"$IP_MARKERS"); ling-mem.prev: $([ -n "$IP_PREV_SHA" ] && echo "v$(ver_of "$STANDIN.prev")" || echo none)"
  [ "$ok" = 5 ] && IP_SEEDED=1
}

# The release serves: run once after the upgrade restarted the engine.
inplace_after() {
  local p="in place (upgrade)" v new marker named
  v="$(ver_of "$LING")"
  engine_mem_health 60
  check "$p: memory answers through the release (/mem/api/health)" $? "no ok from $ENGINE_MEM/api/health" "memory v$(ip_mem_version), engine v$(ip_engine_version)"
  ip_no_daemon_check "$p"
  ip_inode_check "$p"
  ip_lock_check "$p"

  new="$(ip_new_backups)"; marker="$LG/memory/backups/takeover-$v.done"
  if grep -qx "takeover-$v.done" "$IP_MARKERS"; then
    gap "$p: one first-start backup for v$v" "the baseline already holds takeover-$v.done (it ran v$v before) — no backup is due"
    ip_no_new_backup_check "$p" "takeover-$v.done was there before"
  else
    named="$(basename "$(head -1 "$marker" 2>/dev/null)" 2>/dev/null)"
    [ -n "$new" ] && [ "$new" = "${new%% *}" ] && [ "$named" = "$new" ]
    check "$p: exactly one first-start backup for v$v, named by takeover-$v.done" $? \
      "new backups: ${new:-none}; marker $([ -f "$marker" ] && echo "names ${named:-nothing}" || echo missing)" "$new"
  fi

  ip_rows_check "$p" "$ENGINE_MEM"
  "$LING" mem session-start --format json >"$OUT/inplace-session-start.json" 2>&1 && grep -qF "$TK_CORE" "$OUT/inplace-session-start.json"
  check "$p: ling mem session-start loads the core row" $? "$(head -c 200 "$OUT/inplace-session-start.json")"
  check_standin "$p" "$v" "$IP_PREV_SHA"
  [ -n "$IP_PREV_SHA" ] || { [ ! -e "$STANDIN.prev" ]; check "$p: no ling-mem.prev appears" $? "$(ls -l "$STANDIN.prev" 2>&1)"; }
  mcp_answers "$ENGINE_MEM/mcp"
  check "$p: $EP/mem/mcp answers" $? "$MCP_DETAIL" "$MCP_DETAIL"
  mcp_answers "http://127.0.0.1:$EP/mcp/memory"
  check "$p: $EP/mcp/memory answers" $? "$MCP_DETAIL" "$MCP_DETAIL"
  ip_snapshot
}

# A row written by the release, after upgrade.sh's store checks counted rows.
inplace_writes() {
  tk_add "$ENGINE_MEM" "release" semantic "Gate in-place row through the release ($TK_STAMP): Alex mends nets on Sundays."
  check "in place: a row written through the release" $? "no id back" "$(tail -1 "$TK_IDS" | cut -f1)"
  awk -F'\t' '$3 == "release" {print $1}' "$TK_IDS" >"$IP_RELEASE_IDS"
}

# `ling update --rollback` restarted the engine on ling.prev (it hosts memory).
inplace_rollback() { # prev-version
  local p="ling update --rollback" new marker
  ip_wait_engine_version "$1" 90
  engine_mem_health 90
  check "$p: memory answers through v$1 (/mem/api/health)" $? "no ok from $ENGINE_MEM/api/health — $(tail -2 "$OUT/ling-rollback.log" 2>/dev/null)" "memory v$(ip_mem_version)"
  ip_no_daemon_check "$p"
  ip_inode_check "$p"
  ip_lock_check "$p"
  tk_find_all "$ENGINE_MEM" "$IP_RELEASE_IDS"
  check "$p: the row written by the release is there on v$1" $? "missing:$TK_MISSING" "$TK_FOUND ids"
  ip_rows_check "$p" "$ENGINE_MEM"
  if grep -qx "takeover-$1.done" "$IP_MARKERS"; then
    ip_no_new_backup_check "$p" "takeover-$1.done was there: v$1 backs nothing up again"
  else
    new="$(ip_new_backups)"; marker="$LG/memory/backups/takeover-$1.done"
    [ -f "$marker" ] && [ -n "$new" ] && [ "$new" = "${new%% *}" ]
    check "$p: v$1's own first-start backup (it never ran on this store)" $? "new backups: ${new:-none}; marker $([ -f "$marker" ] && echo present || echo missing)" "$new"
  fi
  tk_add "$ENGINE_MEM" "rollback" semantic "Gate in-place row during the rollback ($TK_STAMP): Alex repainted the skiff."
  check "$p: v$1 writes a row" $? "no id back"
  ip_snapshot
}

# `ling update --rollback` again: the release back on the same store.
inplace_forward() {
  local p="ling update --rollback again" v
  v="$(ver_of "$LING")"
  ip_wait_engine_version "$v" 90
  engine_mem_health 60
  check "$p: memory answers through the release" $? "no ok from $ENGINE_MEM/api/health"
  ip_inode_check "$p"
  tk_find_all "$ENGINE_MEM" "$TK_ALL"
  check "$p: every row is found through the release, the rollback's included" $? "missing:$TK_MISSING" "$TK_FOUND ids"
  ip_no_new_backup_check "$p" "takeover-$v.done holds: no second backup for v$v"
}
