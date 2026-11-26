# App-first and plugin-first each get a fresh VM. Script-first runs in first-install.sh.
. ./common.sh
. ./expect.env
[ -n "$APP_VERSION" ] || { gap "$ORDER" "no app in the mirror"; exit 0; }

app_phase() {
  copy_app_release "$APP_VERSION" || { fail "$ORDER: copy bundle" "could not stage the release bundle"; return 1; }
  open_app "$APP_BUNDLE"
  engine_health 120
  check "$ORDER: app starts the engine" $? "no health after first launch"
  one_ling "$ORDER: app"
}

plugin_phase() {
  # Use the actual release bundle's SessionStart hook, without needing a
  # logged-in Claude Code session. first-install.sh checks host registration.
  local root="$OUT/bundle/plugins/linggen" i
  env $RB CLAUDE_PLUGIN_ROOT="$root" LINGGEN_START_WAIT=0 \
    bash "$root/hooks/autostart.sh" </dev/null >"$OUT/plugin-hook.log" 2>&1
  for ((i = 0; i < 180; i++)); do
    [ ! -d "$LG/.engine-install.lock" ] && break
    sleep 2
  done
  [ ! -f "$LG/engine-install.log" ] || cp "$LG/engine-install.log" "$OUT/plugin-engine-install.log"
  [ "$(app_version)" = "$APP_VERSION" ]
  check "$ORDER: plugin uses the release app" $? "got '$(app_version)', want $APP_VERSION"
  engine_health 90
  check "$ORDER: plugin starts the engine" $? "$(tail -3 "$LG/engine-install.log" 2>/dev/null)"
  one_ling "$ORDER: plugin"
}

[ -n "$RB" ] && launchctl setenv LINGGEN_RELEASE_BASE "$GOOD"
case "$ORDER" in
  app-script-plugin)
    app_phase || exit 1
    pid="$(listener_pid "$EP")"
    install_app_release "$OUT/install-existing.log" # unpinned: must only repair the link
    check "$ORDER: script exits 0" $? "$(tail -3 "$OUT/install-existing.log")"
    one_ling "$ORDER: script"
    [ -n "$pid" ] && [ "$pid" = "$(listener_pid "$EP")" ] && ! grep -q '^Downloading ' "$OUT/install-existing.log"
    check "$ORDER: script does not replace the running app" $? "engine pid changed or a download was attempted"
    plugin_phase
    ;;
  plugin-app)
    plugin_phase
    stop_engine
    app_phase || exit 1
    ;;
  *) fail "install order" "unknown order $ORDER"; exit 1 ;;
esac
quit_app
launchctl unsetenv LINGGEN_RELEASE_BASE
exit 0
