# Release gate — helpers for the scripts that run inside the VM. Each check
# prints one line the host turns into a record:
#   RESULT<TAB>STATUS<TAB>check<TAB>detail
# A fresh macOS has curl, jq, shasum, plutil, spctl — no python3 (its stub
# would raise the Command Line Tools dialog), so nothing here uses it.

set -uo pipefail
# The last line out, whatever the exit: the host FAILs a run without it
# (lib.sh vm_run), so a set -u abort or a crash can't leave the gate green.
trap 'printf "GATE_EXIT\t%s\n" "$?"' EXIT
. ./telemetry.sh || exit 1
gate_disable_telemetry
export PATH="$HOME/.local/bin:/usr/local/bin:$PATH"

GOOD="$MIRROR/good"
RELABEL="$MIRROR/relabel"
BAD="$MIRROR/bad"
LG="$HOME/.linggen"
OUT="$HOME/gate"

# --public: the real linggen.dev and GitHub Latest, no LINGGEN_RELEASE_BASE —
# a person's install. Otherwise the mirror. RB is `env`'s argument.
if [ "${GATE_SOURCE:-}" = public ]; then
  SITE=https://linggen.dev RB=""
  asset_url() { echo "https://github.com/$1/releases/latest/download/$2"; }
else
  SITE="$GOOD" RB="LINGGEN_RELEASE_BASE=$GOOD"
  asset_url() { echo "$GOOD/$1/$2"; }
fi

r() { printf 'RESULT\t%s\t%s\t%s\n' "$1" "$2" "$(printf '%s' "${3:-}" | tr '\t\n' '  ' | cut -c1-300)"; }
pass() { r PASS "$@"; }
fail() { r FAIL "$@"; }
warn() { r WARN "$@"; }
gap() { r GAP "$@"; }
info() { r INFO "$@"; }
check() { # name rc fail-detail [pass-detail]
  if [ "$2" = 0 ]; then pass "$1" "${4:-}"; else fail "$1" "$3"; fi
}

open_app() { # app path
  if ! gate_vm_telemetry_disabled; then
    fail "app: telemetry disabled" 'guest is missing ~/.linggen/no-telemetry'
    exit 1
  fi
  open "$@"
}

wait_http() { # url seconds
  local i
  for ((i = 0; i < $2; i++)); do curl -fsS -m 2 "$1" >/dev/null 2>&1 && return 0; sleep 1; done
  return 1
}

sha() { shasum -a 256 "$1" 2>/dev/null | awk '{print $1}'; }
ver_of() { "$1" --version 2>/dev/null | awk '{print $2}'; }
ling_bin() { [ -z "${GATE_STANDALONE:-}" ] || { printf '%s\n' "$GATE_STANDALONE"; return; }; command -v ling || echo "$HOME/.local/bin/ling"; }
APP_BUNDLE=/Applications/Linggen.app
APP_LING="$APP_BUNDLE/Contents/MacOS/ling"

one_ling() { # phase label
  local path found resolved count=0 real=""
  hash -r
  found="$(command -v ling)"
  resolved="$(realpath "$found" 2>/dev/null)"
  for path in /usr/local/bin/ling "$HOME/.local/bin/ling" "$APP_LING"; do
    if [ -f "$path" ] && [ ! -L "$path" ]; then
      count=$((count + 1)); real="$path"
    fi
  done
  [ "$count" = 1 ] && [ -x "$found" ] && [ "$resolved" = "$(realpath "$real")" ]
  check "$1: one_ling" $? "$count real files; ${found:-no command} resolves to ${resolved:-nothing}" "$found → $resolved"
}

install_app_release() { # log [installer arguments]
  local log="$1"; shift
  curl -fsSL "$SITE/install.sh" | env $RB bash -s -- "$@" >"$log" 2>&1
}

# Drag-to-Applications equivalent: the release tarball holds the same bundle
# as the DMG. Use a fresh bundle, even if a script already installed it.
copy_app_release() { # version
  local stage
  stage="$(mktemp -d "$OUT/app-copy.XXXXXX")" || return 1
  curl -fsSL "$(asset_url linggen/linggen-releases "linggen-$1-darwin-arm64.tar.gz")" \
    | tar -xzf - -C "$stage" || return 1
  [ -x "$stage/Linggen.app/Contents/MacOS/ling" ] || return 1
  rm -rf "$APP_BUNDLE" && mv "$stage/Linggen.app" "$APP_BUNDLE"
}

check_app_identity() {
  local health exe commit sub rc line
  health="$(curl -fsS -m 5 "http://127.0.0.1:$EP/api/health")"
  exe="$(jq -r '.exe // empty' <<<"$health")"
  commit="$(jq -r '.commit // empty' <<<"$health")"
  [ "$exe" = "$(realpath "$APP_LING")" ] && [[ "$commit" =~ ^[0-9a-f]{7,}$ ]]
  check "app: health identifies the bundle engine" $? "exe=${exe:-missing}, commit=${commit:-missing}" "$exe · $commit"
  ling status >"$OUT/status-binary.log" 2>&1
  grep -Fxq "Binary: $exe · $commit" "$OUT/status-binary.log"
  check "app: status agrees with health" $? "$(grep '^Binary:' "$OUT/status-binary.log")"
  for sub in install update; do
    line="$(ling "$sub" 2>&1)"; rc=$?
    [ "$rc" != 0 ] && [ "$line" = 'Linggen.app updates itself' ]
    check "app: ling $sub refuses" $? "exit $rc: $line"
  done
}
mem_bin() { command -v ling-mem || echo "$HOME/.local/bin/ling-mem"; }

engine_health() { wait_http http://127.0.0.1:9527/api/health "${1:-60}"; }
mem_health() { wait_http http://127.0.0.1:9528/api/health "${1:-60}"; }
mem_version() { curl -fsS -m 3 http://127.0.0.1:9528/api/health 2>/dev/null | jq -r '.data.version // empty'; }

# ── Memory inside the engine ──────────────────────────────────────────────
# The engine serves memory at /mem/* and, for hosts, at its one MCP door /mcp
# on its own port; the retired /mcp/memory only answers 410 "update the
# plugin". Nothing listens on 127.0.0.1:9528 any more. EP is the engine's port, LP the
# old ling-mem daemon's (a 2.2.1 baseline and a rollback to it serve there);
# other values only for a rehearsal outside a VM.
EP="${GATE_ENGINE_PORT:-9527}"
LP="${GATE_LEGACY_PORT:-9528}"
ENGINE_MEM="http://127.0.0.1:$EP/mem"
STANDIN_MARK='# linggen-ling-mem-stand-in'

engine_mem_health() { # seconds
  local i
  for ((i = 0; i < ${1:-30}; i++)); do
    curl -fsS -m 3 "$ENGINE_MEM/api/health" 2>/dev/null | jq -e '.ok == true' >/dev/null 2>&1 && return 0
    sleep 1
  done
  return 1
}

mem_post() { # base path json → memory's envelope
  curl -sS -m 120 -X POST -H 'content-type: application/json' -d "$3" "$1$2" 2>&1
}

# initialize + tools/list on an MCP URL, listing TOOL (default memory_search);
# MCP_DETAIL says what answered, MCP_COUNT and MCP_NAMES what it listed.
mcp_answers() { # url [tool]
  local init list n want="${2:-memory_search}"
  init="$(mem_post "$1" "" '{"jsonrpc":"2.0","id":1,"method":"initialize","params":{"protocolVersion":"2025-03-26","capabilities":{},"clientInfo":{"name":"linggen-gate","version":"1"}}}')"
  list="$(mem_post "$1" "" '{"jsonrpc":"2.0","id":2,"method":"tools/list","params":{}}')"
  n="$(jq -r '[.result.tools[]?.name] | length' <<<"$list" 2>/dev/null)"
  MCP_COUNT="$n"; MCP_NAMES="$(jq -r '.result.tools[]?.name' <<<"$list" 2>/dev/null)"
  MCP_DETAIL="server $(jq -r '.result.serverInfo.name // "?"' <<<"$init" 2>/dev/null) $(jq -r '.result.serverInfo.version // ""' <<<"$init" 2>/dev/null), $n tools"
  jq -e '.result.protocolVersion' >/dev/null 2>&1 <<<"$init" \
    && jq -e --arg w "$want" '[.result.tools[]?.name] | index($w) != null' >/dev/null 2>&1 <<<"$list" && return 0
  MCP_DETAIL="initialize: $(head -c 120 <<<"$init") / tools/list: $(head -c 120 <<<"$list")"
  return 1
}

# The retired memory door: refused with 410 and code UPGRADE_PLUGIN, never
# served — what a host with the old plugin's `memory` server gets.
# RETIRED_DETAIL says what answered.
mcp_retired() { # url
  local out code body
  out="$(curl -sS -m 10 -w '\n%{http_code}' -X POST -H 'content-type: application/json' \
    -d '{"jsonrpc":"2.0","id":1,"method":"initialize","params":{"protocolVersion":"2025-03-26","capabilities":{},"clientInfo":{"name":"linggen-gate","version":"1"}}}' "$1" 2>&1)"
  code="${out##*$'\n'}"; body="${out%$'\n'*}"
  RETIRED_DETAIL="HTTP $code: $(head -c 160 <<<"$body")"
  [ "$code" = 410 ] && jq -e '.code == "UPGRADE_PLUGIN"' >/dev/null 2>&1 <<<"$body"
}

# The one door's tools are the catalogue's (src/server/mcp/catalog.rs). The
# host hands in their names, in order, as DOOR_TOOL_NAMES="a,b,c": read from
# the plugin evals' _tools.json, which a test locks to the catalogue — so the
# gate holds no count of its own. MCP_NAMES (mcp_answers) must be exactly them.
door_lists_the_catalogue() {
  [ -n "${DOOR_TOOL_NAMES:-}" ] && [ "$(tr '\n' ',' <<<"$MCP_NAMES")" = "$DOOR_TOOL_NAMES," ]
}

is_standin() { head -c 300 "$1" 2>/dev/null | grep -qF "$STANDIN_MARK"; }

# Real ling-mem daemons still running (the stand-in is a script: its process is sh).
ling_mem_procs() { { pgrep -x ling-mem; pgrep -x ling-mem.prev; } 2>/dev/null | tr '\n' ' '; }

# Nothing listens on a port. PORT_DETAIL says what does, if anything.
nothing_on() { # port
  local lines pids addrs
  lines="$(lsof -nP -iTCP:"$1" -sTCP:LISTEN -Fpn 2>/dev/null)"
  pids="$(grep '^p' <<<"$lines" | cut -c2- | sort -u | tr '\n' ' ')"
  addrs="$(grep '^n' <<<"$lines" | cut -c2- | sort -u | tr '\n' ' ')"
  PORT_DETAIL="nothing on $1"
  [ -z "$pids" ] && return 0
  PORT_DETAIL="pid ${pids}on ${addrs:-?}"
  return 1
}

# The engine's pid is the one listener on every address of its port.
# PORT_DETAIL lists what listens.
engine_alone_on_its_port() { # engine-pid
  local lines pids
  lines="$(lsof -nP -iTCP:"$EP" -sTCP:LISTEN -Fp 2>/dev/null)"
  pids="$(grep '^p' <<<"$lines" | cut -c2- | sort -u | tr '\n' ' ')"
  PORT_DETAIL="pid ${pids:-none}on $EP (engine pid ${1:-none})"
  [ -n "$1" ] && [ "$pids" = "$1 " ]
}

# The stand-in at ~/.local/bin/ling-mem, and the binary it moved aside.
# want-prev-sha empty: nothing was there to move (no ling-mem.prev expected).
check_standin() { # label want-version want-prev-sha
  local bin="$HOME/.local/bin/ling-mem" v port
  v="$(ver_of "$bin")"
  is_standin "$bin" && [ "$v" = "$2" ]
  check "$1: ~/.local/bin/ling-mem is the stand-in" $? "$(head -c 80 "$bin" 2>/dev/null | tr '\n' ' ') — answers v${v:-none}, want the engine's v$2" "answers v$v"
  port="$("$bin" status 2>/dev/null | jq -r '.port // empty' 2>/dev/null)"
  [ "$port" = "$EP" ]
  check "$1: the stand-in's status names the engine's port" $? "status says port ${port:-none}, want $EP" "port $port"
  if [ -n "$3" ]; then
    [ "$(sha "$bin.prev")" = "$3" ]
    check "$1: ling-mem.prev is the old binary" $? "sha $(sha "$bin.prev" | cut -c1-12) (v$(ver_of "$bin.prev")), want ${3:0:12}" "v$(ver_of "$bin.prev"), sha ${3:0:12}"
  fi
}

# The installed binary is the one the release carries (versions can match a
# published release in --local runs, so compare bytes).
# The app build signs its own copy of ling; standalone updaters do not use it.
release_ling_sha() { # install kind
  case "$1" in
    app) printf '%s\n' "$APP_LING_SHA" ;;
    standalone) printf '%s\n' "$LING_SHA" ;;
    *) return 1 ;;
  esac
}

check_installed() { # label bin install-kind
  local v s want version="$LING_VERSION"
  want="$(release_ling_sha "$3")"
  if [ "$3" = app ] && [ "${APP_IN_TRAIN:-1}" != 1 ]; then
    # A published fallback app may bundle a different engine version.
    version="$(ver_of "$APP_LING")"
  fi
  v="$(ver_of "$2")"; s="$(sha "$2")"
  if [ -n "$version" ] && [ "$v" = "$version" ] && [ -n "$want" ] && [ "$s" = "$want" ]; then pass "$1" "$v at $2 ($3 sha ${want:0:12})"
  else fail "$1" "got $v (sha ${s:0:12}) at $2, want $version ($3 sha ${want:0:12})"; fi
}

start_engine() { # bare `ling` — the daemon plus the browser, as a person runs it
  (cd "$HOME" && nohup "$(ling_bin)" >"$OUT/ling-start.log" 2>&1 </dev/null &)
  engine_health "${1:-60}"
}

stop_engine() {
  "$(ling_bin)" stop >/dev/null 2>&1 || true
  pkill -f '/ling --web' 2>/dev/null || true
  sleep 1
}

start_mem() {
  mem_health 2 && return 0
  (nohup "$(mem_bin)" serve --port 9528 >"$OUT/ling-mem.log" 2>&1 </dev/null &)
  mem_health "${1:-60}"
}

stop_mem() {
  local pid
  pid="$(lsof -ti tcp:9528 -sTCP:LISTEN 2>/dev/null)"
  [ -n "$pid" ] && kill $pid 2>/dev/null
  sleep 1
}

# The gate's fake model (llmposter on the host, tunnelled to $MODEL) as the
# only model, so a chat turn is deterministic and never reaches a provider.
write_fake_model_config() {
  mkdir -p "$LG/config"
  [ -f "$LG/config/linggen.runtime.toml" ] && cp "$LG/config/linggen.runtime.toml" "$OUT/runtime.toml.before-gate"
  sed "s#{{MODEL_URL}}#$MODEL#g" "$OUT/gate.runtime.toml" >"$LG/config/linggen.runtime.toml"
}

# One turn: a new session, "gate-ping", wait for the fake model's line in the
# session's messages. 0 = the reply landed.
chat_turn() { # seconds
  local sid i
  sid="$(curl -fsS -m 30 -X POST -H 'content-type: application/json' \
    -d "{\"title\":\"gate\",\"project_root\":\"$HOME\"}" http://127.0.0.1:9527/api/sessions | jq -r '.id // empty')"
  [ -n "$sid" ] || { CHAT_DETAIL="session create failed"; return 1; }
  curl -fsS -m 30 -X POST -H 'content-type: application/json' \
    -d "{\"project_root\":\"$HOME\",\"session_id\":\"$sid\",\"message\":\"gate-ping\"}" \
    http://127.0.0.1:9527/api/chat >"$OUT/chat-post.json" 2>&1 \
    || { CHAT_DETAIL="POST /api/chat: $(head -c 200 "$OUT/chat-post.json")"; return 1; }
  for ((i = 0; i < ${1:-90}; i++)); do
    if grep -q 'GATE-PONG-7731' "$LG/sessions/$sid/messages.jsonl" 2>/dev/null; then
      CHAT_DETAIL="session $sid"; return 0
    fi
    sleep 1
  done
  CHAT_DETAIL="no reply in $LG/sessions/$sid/messages.jsonl after ${1:-90}s"
  return 1
}

# TCC: watch tccd while the first-run path runs; a prompt shown = fail.
tcc_watch_start() {
  log stream --style compact --predicate 'subsystem == "com.apple.TCC"' >"$OUT/tcc.log" 2>&1 &
  echo $! >"$OUT/tcc.pid"
  sleep 2
}

# Linggen's requests are the msgIDs whose attribution names one of our
# binaries. preflight=yes is a silent check; a preflight=no request answered by
# the user (authReason 2), timed out (9), or never answered is a prompt shown.
TCC_OURS='/\.local/bin/ling|/Applications/Linggen\.app/|/\.linggen/'

tcc_watch_stop() { # label
  local pid; pid="$(cat "$OUT/tcc.pid" 2>/dev/null)"
  [ "${pid:-0}" -gt 1 ] 2>/dev/null && kill "$pid" 2>/dev/null
  local log="$OUT/tcc.log" m ctx res seen="" prompts="" line
  for m in $(grep -E "AUTHREQ_ATTRIBUTION" "$log" | grep -E "$TCC_OURS" | grep -oE 'msgID=[0-9.]+' | sort -u); do
    ctx="$(grep -F "AUTHREQ_CTX: $m," "$log" | head -1)"
    res="$(grep -F "AUTHREQ_RESULT: $m," "$log" | head -1)"
    line="$(grep -oE 'kTCCService[A-Za-z]+' <<<"$ctx")($(grep -oE 'preflight=[a-z]+' <<<"$ctx" | cut -d= -f2),$(grep -oE 'authValue=[0-9]+' <<<"$res" | cut -d= -f2)/$(grep -oE 'authReason=[0-9]+' <<<"$res" | cut -d= -f2))"
    seen="$seen $line"
    if grep -q 'preflight=no' <<<"$ctx" && { [ -z "$res" ] || grep -qE 'authReason=(2|9),' <<<"$res"; }; then
      prompts="$prompts $line"
    fi
  done
  grep -q 'AUTHREQ_PROMPTING' "$log" && prompts="$prompts AUTHREQ_PROMPTING:$(grep -m1 AUTHREQ_PROMPTING "$log" | cut -c1-160)"
  info "$1: Linggen's TCC requests" "$(tr ' ' '\n' <<<"$seen" | sort | uniq -c | awk 'NF{printf "%s%s ", $2, ($1>1?"x"$1:"")}')"
  if [ -n "$prompts" ]; then fail "$1: no TCC prompt" "prompted:$prompts"
  else pass "$1: no TCC prompt" "$(wc -l <"$log" | tr -d ' ') tccd lines; Linggen's requests all silent (preflight or policy)"; fi
}

# The Command Line Tools install dialog: what running the /usr/bin/git stub
# (or any other CLT shim) opens on a Mac without developer tools.
CLT_DIALOG='Install Command Line Developer Tools|CommandLineTools\.installondemand'
clt_dialog_up() { pgrep -f "$CLT_DIALOG" >/dev/null 2>&1; }
# SIGKILL: the dialog ignores SIGTERM, and a dialog left up is blamed on the
# next check that looks for one.
clt_dialog_close() { pkill -9 -f "$CLT_DIALOG" 2>/dev/null || true; sleep 1; }

listener_pid() { lsof -nP -ti "tcp:$1" -sTCP:LISTEN 2>/dev/null | head -1; }

# install-plugin.sh from the mirror (or linggen.dev), which installs from the release bundle.
install_plugin() { # log
  curl -fsSL "$SITE/install-plugin.sh" | env $RB bash >"$1" 2>&1
}

# The plugin is linggen@linggen (this repo's marketplace). The id is a prefix
# of the old linggen@linggen-memory, so every match on it excludes that one.
PLUGIN_ID=linggen@linggen
OLD_PLUGIN_ID=linggen@linggen-memory
NEW_ID_RE='linggen@linggen([^-[:alnum:]]|$)'

# The block `claude plugin list` prints for one plugin id (an ERE).
claude_plugin_block() { # id-re
  claude plugin list 2>/dev/null | awk -v re="$1" '/❯/{f = ($0 ~ re)} f'
}

# The folder Claude Code loads the linggen plugin from: "Read from:" for a
# folder marketplace, else its cache copy.
claude_plugin_root() {
  local root
  root="$(claude_plugin_block "$NEW_ID_RE" | awk '/Read from:/{print $NF; exit}')"
  [ -n "$root" ] || root="$(ls -td "$HOME"/.claude/plugins/cache/linggen/linggen/*/ 2>/dev/null | head -1)"
  printf '%s' "${root%/}"
}

# The plugin checks after install-plugin.sh, shared by both phases: the new
# plugin enabled in each host, the old one gone, the stand-in kept and
# nothing on the old memory port.
check_plugins() { # label rc log
  local root left list
  check "$1: install-plugin.sh exits 0" "$2" "$(tail -3 "$3")"
  sleep 3
  clt_dialog_up; [ $? != 0 ]
  check "$1: no Command Line Tools dialog" $? "the CLT install dialog is up after install-plugin.sh"
  if command -v claude >/dev/null; then
    root="$(claude_plugin_root)"
    claude_plugin_block "$NEW_ID_RE" | grep -q enabled
    check "$1: claude has $PLUGIN_ID enabled" $? "$(claude plugin list 2>&1 | tr '\n' ' ' | cut -c1-200)" "$root"
    [ -z "$(claude_plugin_block "$OLD_PLUGIN_ID")" ]
    check "$1: claude has no $OLD_PLUGIN_ID" $? "$(claude_plugin_block "$OLD_PLUGIN_ID" | tr '\n' ' ' | cut -c1-200)"
    [ -n "$root" ] && diff -rq "$root/hooks" "$OUT/bundle/plugins/linggen/hooks" >/dev/null
    check "$1: claude loads this release's hooks" $? "hooks at '${root:-none}' differ from the release's"
    [ -n "$root" ] && jq -e '(.mcpServers | keys) == ["linggen"] and (.mcpServers.linggen.url | endswith("/mcp"))' "$root/.mcp.json" >/dev/null 2>&1
    check "$1: claude's plugin declares one linggen server at /mcp" $? "$(jq -c '.mcpServers | map_values(.url)' "$root/.mcp.json" 2>&1 | head -c 200)"
  else
    gap "$1: claude plugin" "no claude CLI here"
  fi
  if command -v codex >/dev/null; then
    list="$(codex plugin list 2>&1)"
    grep -E "$NEW_ID_RE" <<<"$list" | grep -q 'enabled'
    check "$1: codex has $PLUGIN_ID enabled" $? "$(grep -i linggen <<<"$list" | tr '\n' ' ' | cut -c1-200)"
    ! grep -qF "$OLD_PLUGIN_ID" <<<"$list"
    check "$1: codex has no $OLD_PLUGIN_ID" $? "$(grep -F "$OLD_PLUGIN_ID" <<<"$list" | tr '\n' ' ' | cut -c1-200)"
  else
    gap "$1: codex plugin" "no codex CLI here"
  fi
  # The plugin's start hook ran: the engine's stand-in must stay, no daemon
  # run beside it, and nothing listen on the old memory port.
  sleep 2; left="$(ling_mem_procs)"
  [ -z "$left" ] && is_standin "$HOME/.local/bin/ling-mem"
  check "$1: after the plugins, the stand-in stays and no ling-mem daemon runs" $? \
    "ling-mem processes: ${left:-none}; ~/.local/bin/ling-mem: $(head -c 60 "$HOME/.local/bin/ling-mem" 2>/dev/null | tr '\n' ' ')"
  nothing_on "$LP"
  check "$1: after the plugins, nothing listens on the old port $LP" $? "$PORT_DETAIL" "$PORT_DETAIL"
}

app_version() { plutil -extract CFBundleShortVersionString raw /Applications/Linggen.app/Contents/Info.plist 2>/dev/null; }

# ── 灵境's asset pack (the engine's doc/asset-pack-spec.md) ────────────────
# The app ships 灵境 without the files its assets.json lists; the engine
# fetches them by hash into ~/.linggen/assets/sha256/ and GET /api/assets/
# lingjing counts what is held. `held` also counts a file the skill folder
# holds, so the check proves a fetch only when the folder lacks them.
ASSET_DIR="$LG/skills/lingjing"
ASSET_STORE="$LG/assets/sha256"

assets_in_folder() { # manifest paths the installed skill folder holds
  local p n=0
  while IFS= read -r p; do [ -f "$ASSET_DIR/$p" ] && n=$((n + 1)); done \
    < <(jq -r '.files[].path' "$ASSET_DIR/assets.json" 2>/dev/null)
  echo "$n"
}
assets_in_store() { ls "$ASSET_STORE" 2>/dev/null | grep -vc '\.part$'; }

# The first listed object's URL, by the engine's rule: <base><2 hex>/<sha>[.<ext>].
assets_probe_url() {
  jq -r '.base as $b | .files[0] | $b + .sha256[0:2] + "/" + .sha256
    + (.path | if test("\\.[A-Za-z0-9]{1,8}$") then "." + (split(".") | last) else "" end)' "$ASSET_DIR/assets.json"
}

# label seconds mode t0 — mode: first (the folder must lack every listed file)
# or upgrade (a baseline copy that keeps them is a GAP for the fetch). t0: $SECONDS
# when the app was opened. Run while the app's engine is up.
check_assets_heal() {
  local label="$1" wait="$2" mode="$3" t0="$4" total folder store0 url code rep held failing i
  if [ ! -f "$ASSET_DIR/assets.json" ]; then
    fail "$label: 灵境 carries assets.json" "no $ASSET_DIR/assets.json — $(ls "$ASSET_DIR" 2>/dev/null | head -c 120)"
    return
  fi
  total="$(jq '.files | length' "$ASSET_DIR/assets.json")"
  folder="$(assets_in_folder)"; store0="$(assets_in_store)"
  url="$(assets_probe_url)"
  code="$(curl -sS -o /dev/null -I -m 20 -w '%{http_code}' "$url" 2>/dev/null)"
  if [ "$code" = 000 ] || [ -z "$code" ]; then
    gap "$label: 灵境's pictures fetched after install" "the VM cannot reach $url — needs outside network to assets.linggen.dev"
    return
  fi
  [ "$code" = 200 ]
  check "$label: the asset service serves 灵境's objects" $? "HEAD $url → $code" "HEAD of the first object → 200"
  if [ "$folder" = 0 ]; then
    pass "$label: the installed 灵境 lacks the manifest's files" "0 of $total in $ASSET_DIR (store held $store0 then)"
  elif [ "$mode" = first ]; then
    fail "$label: the installed 灵境 lacks the manifest's files" "$folder of $total are in $ASSET_DIR — the bundle was not slimmed"
  else
    gap "$label: the installed 灵境 lacks the manifest's files" "$folder of $total still in $ASSET_DIR (marker: $(jq -c '{by,app_version}' "$ASSET_DIR/.installed" 2>/dev/null || echo none)) — the baseline's copy kept them, so no fetch is proved here"
  fi
  for ((i = 0; i < wait; i += 5)); do
    rep="$(curl -fsS -m 5 "http://127.0.0.1:$EP/api/assets/lingjing" 2>/dev/null)"
    held="$(jq -r '.held_files // empty' <<<"$rep" 2>/dev/null)"
    failing="$(jq -r '[.packs[]?.failing] | add // 0' <<<"$rep" 2>/dev/null)"
    [ "$held" = "$total" ] && [ "$failing" = 0 ] && break
    sleep 5
  done
  [ "$held" = "$total" ] && [ "$failing" = 0 ] && [ "$(assets_in_folder)" = "$folder" ]
  check "$label: the engine holds 灵境's pictures" $? \
    "after ${wait}s: held ${held:-?} of $total, failing ${failing:-?}, store $(assets_in_store) objects — $(head -c 160 <<<"${rep:-no report}")" \
    "held $held of $total, 0 failing, $((SECONDS - t0)) s after the app opened; store $store0 → $(assets_in_store) objects, folder $folder"
}

# pkill, not osascript: an Apple Event from sshd would itself be a TCC ask.
quit_app() {
  pkill -TERM -f '/Linggen.app/Contents/MacOS/' 2>/dev/null || true
  sleep 3
  pkill -KILL -f '/Linggen.app/Contents/MacOS/' 2>/dev/null || true
}
