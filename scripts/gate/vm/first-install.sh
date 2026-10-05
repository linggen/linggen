# Release gate — first install on a fresh VM (doc/test-design.md § 4).
# Env from the host: MIRROR (http://127.0.0.1:18765), MODEL, and expect.env.
. ./common.sh
. ./expect.env

[ ! -e "$LG" ] && pass "fresh VM (no ~/.linggen)" || fail "fresh VM (no ~/.linggen)" "$LG exists"

# The whole first-run path runs under a tccd watch.
tcc_watch_start

# ── install.sh — the linggen.dev one-liner, pointed at the mirror ──────────
curl -fsSL "$GOOD/install.sh" | LINGGEN_RELEASE_BASE="$GOOD" bash >"$OUT/install.log" 2>&1
check "install.sh exits 0" $? "exit $? — $(tail -3 "$OUT/install.log")"
check_installed "install.sh: ling is the release" "$(ling_bin)" "$LING_VERSION" "$LING_SHA"
check_installed "install.sh: ling-mem is the release" "$(mem_bin)" "$MEM_VERSION" "$MEM_SHA"
grep -q 'Verified SHA-256' "$OUT/install.log"
check "install.sh: verifies ling's sha256" $? "no 'Verified SHA-256' line"

# ── install-shared-memory.sh — retired: says so and exits 0 ────────────────
curl -fsSL "$GOOD/install-shared-memory.sh" | bash >"$OUT/install-shared-memory.log" 2>&1
rc=$?
grep -q 'retired' "$OUT/install-shared-memory.log"
check "install-shared-memory.sh: retired notice, exit 0" $(( rc + $? )) "exit $rc — $(tail -2 "$OUT/install-shared-memory.log")"

# ── First run, as a person: bare `ling` (daemon + browser) ─────────────────
start_engine 90
check "ling: 9527 /api/health" $? "no health after 90 s — $(tail -3 "$LG/ling.log" 2>/dev/null)"
page="$(curl -fsS -m 10 http://127.0.0.1:9527/ 2>/dev/null)"
grep -qiE '<div id="root"|<script' <<<"$page"
check "ling: UI page served" $? "GET / gave $(printf '%s' "$page" | head -c 120)" "$(printf '%s' "$page" | wc -c | tr -d ' ') bytes"
sleep 5
pgrep -qx Safari && info "ling: browser opened" "Safari running" || info "ling: browser opened" "no Safari process"

# ling-mem: the engine starts its daemon; session_start must answer.
# The engine starts ling-mem on first use (memory_http.rs autostart), or at
# boot only when a skill uses scoped memory — a fresh install has none.
if mem_health 20; then info "ling-mem: 9528 at first run" "up, v$(mem_version)"
else info "ling-mem: 9528 at first run" "not started yet (lazy); starting it by hand"; start_mem 60; fi
"$(mem_bin)" session-start --format json >"$OUT/session-start.json" 2>&1
check "ling-mem: session_start answers" $? "$(head -c 200 "$OUT/session-start.json")" "$(head -c 80 "$OUT/session-start.json")"

# ── One chat turn on the fake model ────────────────────────────────────────
stop_engine
write_fake_model_config
start_engine 60 || fail "ling: restart on the fake-model config" "no health"
chat_turn 90
check "chat: one turn completes on the fake model" $? "$CHAT_DETAIL" "$CHAT_DETAIL"
stop_engine

# ── install-app.sh — Linggen.app from the mirror ───────────────────────────
if [ -n "$APP_VERSION" ]; then
  curl -fsSL "$GOOD/install-app.sh" | LINGGEN_RELEASE_BASE="$GOOD" bash -s -- --no-launch >"$OUT/install-app.log" 2>&1
  check "install-app.sh exits 0" $? "$(tail -3 "$OUT/install-app.log")"
  [ "$(app_version)" = "$APP_VERSION" ]
  check "install-app.sh: Linggen.app is the release" $? "got '$(app_version)', want $APP_VERSION" "$APP_VERSION"
  xattr -r /Applications/Linggen.app 2>/dev/null | grep -q com.apple.quarantine
  [ $? != 0 ]; check "install-app.sh: no quarantine on the app" $? "com.apple.quarantine present"

  launchctl setenv LINGGEN_RELEASE_BASE "$GOOD"
  open /Applications/Linggen.app
  engine_health 120
  check "app: first launch brings up 9527" $? "no health 120 s after open"
  pgrep -f '/Linggen.app/Contents/MacOS/' >/dev/null
  check "app: still running after first launch" $? "no Linggen.app process"
  sleep 10
  quit_app
  launchctl unsetenv LINGGEN_RELEASE_BASE
else
  gap "install-app.sh" "no app in the mirror"
fi

tcc_watch_stop "first run"

# ── A browser-style download: quarantine set, as Safari/Chrome would ───────
if [ -n "$APP_VERSION" ]; then
  q="$HOME/Downloads/gate-quarantine"
  rm -rf "$q"; mkdir -p "$q"
  asset="linggen-$APP_VERSION-darwin-arm64.tar.gz"
  curl -fsSL "$GOOD/linggen/linggen-releases/$asset" -o "$q/$asset"
  xattr -w com.apple.quarantine "0083;$(printf '%x' "$(date +%s)");Safari;" "$q/$asset"
  tar -xzf "$q/$asset" -C "$q"
  # Archive Utility carries the archive's quarantine onto what it unpacks.
  xattr -rw com.apple.quarantine "0083;$(printf '%x' "$(date +%s)");Safari;" "$q/Linggen.app"
  # The vanilla image ships with Gatekeeper off; a person's Mac has it on.
  sudo -n spctl --global-enable >/dev/null 2>&1 || sudo -n spctl --master-enable >/dev/null 2>&1
  verdict="$(spctl -a -vv -t exec "$q/Linggen.app" 2>&1 | tr '\n' ' ')"
  authority="$(codesign -dvv "$q/Linggen.app" 2>&1 | grep -m1 '^Authority=' || echo 'Authority=ad-hoc')"
  if grep -q 'security disabled' <<<"$verdict"; then
    gap "quarantined download: Gatekeeper verdict" "Gatekeeper is off in the VM and could not be enabled: $verdict"
  elif grep -q 'accepted' <<<"$verdict"; then
    pass "quarantined download: Gatekeeper accepts" "$verdict"
  elif grep -q 'Developer ID' <<<"$authority"; then
    fail "quarantined download: Gatekeeper accepts" "signed ($authority) yet $verdict"
  else
    warn "quarantined download: Gatekeeper blocks the unsigned app" "$authority; $verdict — a browser download shows the 'damaged/unverified' dialog; install-app.sh strips quarantine"
  fi
fi

# ── Claude Code + Codex plugins: install-plugin.sh on a Mac with no git ────
# A fresh Mac's git is the Command Line Tools stub — running it opens the CLT
# install dialog. install-plugin.sh must see that and install from the
# release bundle, with no dialog.
if [ -x "$OUT/codex" ]; then
  # Codex has no unattended installer without node; the host's CLI is copied in.
  mkdir -p "$HOME/.local/bin" && cp "$OUT/codex" "$HOME/.local/bin/codex"
else
  gap "codex: CLI in the VM" "no codex binary handed in"
fi
if curl -fsSL https://claude.ai/install.sh | bash >"$OUT/claude-install.log" 2>&1 && command -v claude >/dev/null; then
  pass "claude: CLI installs unattended" "$(claude --version 2>/dev/null)"
else
  gap "claude: CLI installs unattended" "$(tail -2 "$OUT/claude-install.log")"
fi

dev="$(xcode-select -p 2>/dev/null)"
info "git on this Mac" "${dev:+developer dir $dev}${dev:-none — /usr/bin/git is the CLT stub}"
clt_dialog_close
curl -fsSL "$GOOD/install-plugin.sh" -o "$OUT/install-plugin.sh"
bash -c ". <(sed -n '/^has_git() {/,/^}/p' '$OUT/install-plugin.sh'); has_git"
seen=$?
if [ -z "$dev" ]; then
  [ "$seen" != 0 ]; check "install-plugin.sh: sees no git (the stub is not git)" $? "has_git said yes with no developer tools"
fi
install_plugin "$OUT/install-plugin.log"
check_plugins "first install" $? "$OUT/install-plugin.log"

# The manual GitHub form still clones with git: on this Mac it opens the
# dialog. Reported, not failed — install-plugin.sh is the no-git path. Run
# after it: the dialog this opens was still up after a pkill in the first
# gate run, so before it would mask the check above.
if command -v claude >/dev/null; then
  claude plugin marketplace remove linggen-memory >/dev/null 2>&1
  if claude plugin marketplace add linggen/linggen-memory >"$OUT/claude-public.log" 2>&1; then
    info "claude: GitHub marketplace add" "works here (git present)"
  else
    sleep 2
    clt_dialog_up && d="CLT dialog opened" || d="no dialog seen"
    info "claude: GitHub marketplace add needs git" "$d — $(grep -m1 -iE 'xcode|git|error' "$OUT/claude-public.log" | cut -c1-100)"
    clt_dialog_close
    clt_dialog_up && info "CLT dialog after pkill" "still up: $(pgrep -lf "$CLT_DIALOG" | tr '\n' ' ' | cut -c1-160)"
  fi
  install_plugin "$OUT/install-plugin-again.log" # back to the bundle
fi
exit 0
