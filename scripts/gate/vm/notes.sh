# Release gate — Memory V2, the default since 3.0: Linggen's own notes
# (doc/memory-spec.md § Memory V2; engine own_memory/ and memory_host/upgrade.rs).
# Sourced by first-install.sh and upgrade.sh in the VM, and by notes-local.sh
# on the host against a scratch engine — the same checks, so a rehearsal here
# is the gate's own. Needs common.sh (LG, EP, mcp_answers, mem_post, check).
#
#   notes_fresh <label>               a fresh home ends on v2 with nothing to move
#   notes_moved <label> <ids.tsv>     an old v1 home's rows became notes, once
#   notes_stays_v1 <label>            `v1` written in the config stays v1
#   notes_door <label>                the one door lists memory_notes (and the catalogue)
#   notes_prompts <label> yes|no      Ling's and Yinyue's prompts hold the notes section, or not
#
# Where the move's record lives, and what a note looks like on disk, is the
# engine's (own_memory/store.rs, format.rs); these only read the folder.

NOTES="$LG/memory"
NOTES_MOVED="$NOTES/.state/moved-from-v1.json"
# What each prompt says when the person's memory is the notes
# (own_memory/prompt.rs): both, and a line only the companion gets.
NOTES_SECTION='Linggen keeps notes about the person and the work'
NOTES_COMPANION='A life moment worth keeping'

notes_files() { (cd "$NOTES" 2>/dev/null && find person areas projects moments chronicle -name '*.md' 2>/dev/null | sort); }
notes_count() { notes_files | wc -l | tr -d ' '; }
# A note carries its id in its header comment: `<!-- id: <id> · kind: …`.
notes_hold_id() { grep -rqF --include='*.md' --exclude-dir=.state -- "id: $1 " "$NOTES" 2>/dev/null; }

# The engine's door answers `memory_notes` with a note or an error text.
# NOTES_ANSWER is the text; 0 = a real answer, not an error.
notes_call() { # arguments-json
  local out
  out="$(mem_post "http://127.0.0.1:$EP" /mcp "$(jq -nc --argjson a "$1" \
    '{jsonrpc:"2.0",id:3,method:"tools/call",params:{name:"memory_notes",arguments:$a}}')")"
  NOTES_ANSWER="$(jq -r '.result.content[0].text // .error.message // empty' <<<"$out" 2>/dev/null)"
  [ -n "$NOTES_ANSWER" ] || NOTES_ANSWER="$(head -c 160 <<<"$out")"
  jq -e '.result != null and .result.isError != true' >/dev/null 2>&1 <<<"$out"
}

# The system prompt a session of <agent> is built with: GET /api/chat/system-prompt.
notes_prompt() { # agent project-root
  local sid
  sid="$(curl -fsS -m 30 -X POST -H 'content-type: application/json' \
    -d "$(jq -nc --arg r "$2" '{title: "gate-notes", project_root: $r}')" "http://127.0.0.1:$EP/api/sessions" | jq -r '.id // empty')"
  [ -n "$sid" ] || return 1
  curl -fsS -m 30 -G --data-urlencode "project_root=$2" --data-urlencode "agent_id=$1" --data-urlencode "session_id=$sid" \
    "http://127.0.0.1:$EP/api/chat/system-prompt" | jq -r '.system_prompt // empty'
}

# Ling's prompt and Yinyue's. yes: both hold the notes section, Yinyue's the
# companion line, and neither tells v1's memory server protocol. no: neither
# holds the section.
notes_prompts() { # label yes|no [project-root]
  local root="${3:-$HOME}" ling yin ok=0
  ling="$(notes_prompt ling "$root")"; yin="$(notes_prompt yinyue "")"
  if [ "$2" = yes ]; then
    grep -qF "$NOTES_SECTION" <<<"$ling" && grep -qF "$NOTES_SECTION" <<<"$yin" \
      && grep -qF "$NOTES_COMPANION" <<<"$yin" && ! grep -qF 'MCP server `memory`' <<<"$ling$yin"; ok=$?
    check "$1: Ling's and Yinyue's prompts hold the notes section" $ok \
      "ling ${#ling} chars (section: $(grep -cF "$NOTES_SECTION" <<<"$ling")), yinyue ${#yin} chars (section: $(grep -cF "$NOTES_SECTION" <<<"$yin"), companion line: $(grep -cF "$NOTES_COMPANION" <<<"$yin"))" \
      "ling ${#ling} chars, yinyue ${#yin} chars"
  else
    [ -n "$ling" ] && [ -n "$yin" ] && ! grep -qF "$NOTES_SECTION" <<<"$ling$yin"; ok=$?
    check "$1: neither prompt holds the notes section" $ok \
      "ling ${#ling} chars, yinyue ${#yin} chars; the section is in: $(grep -qF "$NOTES_SECTION" <<<"$ling" && echo ling) $(grep -qF "$NOTES_SECTION" <<<"$yin" && echo yinyue)" \
      "ling ${#ling} chars, yinyue ${#yin} chars"
  fi
}

# The one door lists memory_notes, and — when the host handed in the
# catalogue's names (DOOR_TOOL_NAMES, comma-separated, from the plugin evals'
# _tools.json that a test locks to the catalogue) — exactly those, in order.
# On v2 the tool answers; on v1 the door lists it and refuses the call.
notes_door() { # label [v1]
  mcp_answers "http://127.0.0.1:$EP/mcp" memory_notes
  check "$1: the one door lists memory_notes" $? "$MCP_DETAIL" "$MCP_DETAIL"
  if [ -n "${DOOR_TOOL_NAMES:-}" ]; then
    [ "$(tr '\n' ',' <<<"$MCP_NAMES")" = "$DOOR_TOOL_NAMES," ]
    check "$1: the one door lists the catalogue's tools, in order" $? \
      "listed ($MCP_COUNT): $(tr '\n' ' ' <<<"$MCP_NAMES"); catalogue: $(tr ',' ' ' <<<"$DOOR_TOOL_NAMES")" "$MCP_COUNT tools"
  else
    gap "$1: the one door lists the catalogue's tools, in order" "the host handed in no DOOR_TOOL_NAMES"
  fi
  if [ "${2:-}" = v1 ]; then
    notes_call '{"op":"index"}'; [ $? != 0 ] && grep -q 'not served' <<<"$NOTES_ANSWER"
    check "$1: memory_notes is refused (v1 serves no notes)" $? "$NOTES_ANSWER" "$NOTES_ANSWER"
  else
    notes_call '{"op":"index"}'
    check "$1: memory_notes op=index answers" $? "$NOTES_ANSWER" "$(head -c 80 <<<"$NOTES_ANSWER" | tr '\n' ' ')"
  fi
}

# A first install: v2, with no v1 store to move.
notes_fresh() { # label
  [ ! -e "$NOTES_MOVED" ]
  check "$1: no move from v1 is recorded (a fresh home has nothing to move)" $? "$NOTES_MOVED holds: $(head -c 160 "$NOTES_MOVED" 2>/dev/null)" "no .state/moved-from-v1.json"
  [ "$(notes_count)" = 0 ]
  check "$1: no note exists before the person's first" $? "$(notes_count) topic files: $(notes_files | head -3 | tr '\n' ' ')" "0 topic files"
  notes_door "$1"
  notes_prompts "$1" yes
}

# An old v1 home (rows written by the previous release) after the release has
# started on it: the move ran once and recorded itself, every kept row is a
# note by its own id, staged rows are not. ids.tsv is takeover.sh's
# (id<TAB>tier<TAB>written by), which the baseline's rows were noted in.
# A record that was already there before the upgrade (an upgraded VM kept as
# the next baseline, `--save-prev`) leaves the move nothing to do: a GAP.
notes_moved() { # label ids.tsv [record-was-there-before: 1]
  local id tier by bad="" n=0 carried
  if [ "${3:-0}" = 1 ]; then
    gap "$1: the v1 rows were moved into notes" "the baseline already holds .state/moved-from-v1.json (it ran 3.0 before) — no move is due"
    return 0
  fi
  [ -s "$NOTES_MOVED" ] && jq -e . "$NOTES_MOVED" >/dev/null 2>&1
  check "$1: the move from v1 is recorded (.state/moved-from-v1.json)" $? "$(ls "$NOTES/.state" 2>/dev/null | tr '\n' ' '): $(head -c 200 "$NOTES_MOVED" 2>/dev/null)" "$(jq -c . "$NOTES_MOVED" 2>/dev/null | head -c 160)"
  [ "$(notes_count)" -gt 0 ]
  check "$1: notes were made from the old rows" $? "no topic file under $NOTES" "$(notes_count) topic files"
  while IFS=$'\t' read -r id tier by; do
    [ -n "$id" ] || continue
    n=$((n + 1))
    if [ "$tier" = staged ]; then
      notes_hold_id "$id" && bad="$bad staged:$id-became-a-note"
    else
      notes_hold_id "$id" || bad="$bad $tier:$id-not-a-note"
    fi
  done <"$2"
  carried="$n rows seeded"
  [ "$n" -gt 0 ] && [ -z "$bad" ]
  check "$1: a long-term and a core row are notes by their own id, a staged row is not" $? "${bad:-no seeded rows in $2}" "$carried"
}

# `v1` in the config is the old memory and nothing of the new: no record, no
# notes, no section in the prompts, the tool refused at the door.
notes_stays_v1() { # label [project-root]
  [ ! -e "$NOTES_MOVED" ]
  check "$1: no move from v1 is recorded" $? "$NOTES_MOVED holds: $(head -c 160 "$NOTES_MOVED" 2>/dev/null)" "no .state/moved-from-v1.json"
  [ "$(notes_count)" = 0 ]
  check "$1: no note was made from the v1 rows" $? "$(notes_count) topic files: $(notes_files | head -3 | tr '\n' ' ')" "0 topic files"
  notes_door "$1" v1
  notes_prompts "$1" no "${2:-$HOME}"
}
