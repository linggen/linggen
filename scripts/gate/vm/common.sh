# Release gate — helpers for the scripts that run inside the VM. Each check
# prints one line the host turns into a record:
#   RESULT<TAB>STATUS<TAB>check<TAB>detail
# A fresh macOS has curl, jq, shasum, plutil, spctl — no python3 (its stub
# would raise the Command Line Tools dialog), so nothing here uses it.

set -uo pipefail
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

wait_http() { # url seconds
  local i
  for ((i = 0; i < $2; i++)); do curl -fsS -m 2 "$1" >/dev/null 2>&1 && return 0; sleep 1; done
  return 1
}

sha() { shasum -a 256 "$1" 2>/dev/null | awk '{print $1}'; }
ver_of() { "$1" --version 2>/dev/null | awk '{print $2}'; }
ling_bin() { command -v ling || echo "$HOME/.local/bin/ling"; }
mem_bin() { command -v ling-mem || echo "$HOME/.local/bin/ling-mem"; }

engine_health() { wait_http http://127.0.0.1:9527/api/health "${1:-60}"; }
mem_health() { wait_http http://127.0.0.1:9528/api/health "${1:-60}"; }
mem_version() { curl -fsS -m 3 http://127.0.0.1:9528/api/health 2>/dev/null | jq -r '.data.version // empty'; }

# The installed binary is the one the release carries (versions can match a
# published release in --local runs, so compare bytes).
check_installed() { # label bin want-version want-sha
  local v s
  v="$(ver_of "$2")"; s="$(sha "$2")"
  if [ "$v" = "$3" ] && [ "$s" = "$4" ]; then pass "$1" "$v at $2"
  else fail "$1" "got $v (sha ${s:0:12}) at $2, want $3 (sha ${4:0:12})"; fi
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
clt_dialog_close() { pkill -f "$CLT_DIALOG" 2>/dev/null || true; sleep 1; }

listener_pid() { lsof -nP -ti "tcp:$1" -sTCP:LISTEN 2>/dev/null | head -1; }

# install-plugin.sh from the mirror (or linggen.dev), which installs from the release bundle.
install_plugin() { # log
  curl -fsSL "$SITE/install-plugin.sh" | env $RB bash >"$1" 2>&1
}

# The folder Claude Code loads the linggen plugin from: "Read from:" for a
# folder marketplace, else its cache copy.
claude_plugin_root() {
  local root
  root="$(claude plugin list 2>/dev/null | awk '/linggen@linggen-memory/{f=1} f && /Read from:/{print $NF; exit}')"
  [ -n "$root" ] || root="$(ls -td "$HOME"/.claude/plugins/cache/linggen-memory/linggen/*/ 2>/dev/null | head -1)"
  printf '%s' "${root%/}"
}

# The plugin checks after install-plugin.sh, shared by both phases.
check_plugins() { # label rc log
  local root
  check "$1: install-plugin.sh exits 0" "$2" "$(tail -3 "$3")"
  sleep 3
  clt_dialog_up; [ $? != 0 ]
  check "$1: no Command Line Tools dialog" $? "the CLT install dialog is up after install-plugin.sh"
  if command -v claude >/dev/null; then
    root="$(claude_plugin_root)"
    claude plugin list 2>/dev/null | grep -A5 'linggen@linggen-memory' | grep -q enabled
    check "$1: claude has linggen enabled" $? "$(claude plugin list 2>&1 | tr '\n' ' ' | cut -c1-200)" "$root"
    [ -n "$root" ] && diff -rq "$root/hooks" "$OUT/marketplace/plugins/linggen/hooks" >/dev/null
    check "$1: claude loads this release's hooks" $? "hooks at '${root:-none}' differ from the release's"
  fi
  if command -v codex >/dev/null; then
    codex plugin list 2>/dev/null | grep -q 'linggen@linggen-memory.*enabled'
    check "$1: codex has linggen enabled" $? "$(codex plugin list 2>&1 | tail -2 | tr '\n' ' ' | cut -c1-200)"
  fi
}

app_version() { plutil -extract CFBundleShortVersionString raw /Applications/Linggen.app/Contents/Info.plist 2>/dev/null; }

# pkill, not osascript: an Apple Event from sshd would itself be a TCC ask.
quit_app() {
  pkill -TERM -f '/Linggen.app/Contents/MacOS/' 2>/dev/null || true
  sleep 3
  pkill -KILL -f '/Linggen.app/Contents/MacOS/' 2>/dev/null || true
}
