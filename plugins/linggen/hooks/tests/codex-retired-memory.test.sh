#!/usr/bin/env bash
# Codex's SessionStart hook removes the `memory` server an earlier plugin
# wrote into config.toml (a url at the retired memory door), its
# sub-tables with it, and touches nothing else in the file: other tables,
# comments, a `memory` server of the person's own, the `linggen` table.
set -u
here="$(cd "$(dirname "$0")" && pwd)"
plugin="$(cd "$here/../.." && pwd)"
tmp="$(mktemp -d "$here/.tmp.XXXXXX")" || exit 1
trap 'rm -rf "$tmp"' EXIT
export HOME="$tmp/home"
mkdir -p "$HOME/.codex/plugins"
# The hook tells Codex by where the plugin lives.
ln -s "$plugin" "$HOME/.codex/plugins/linggen"
unset CODEX_HOME CLAUDE_CONFIG_DIR LINGGEN_DATA_DIR LINGGEN_HOST LINGGEN_PORT LINGGEN_TOKEN
config="$HOME/.codex/config.toml"

fail() { printf 'FAIL %s\n' "$1" >&2; exit 1; }

# A config an earlier plugin left, with the person's own tables around it.
cp "$here/upgrade-fixture/codex-config.toml" "$config"
cat >"$tmp/want" <<'TOML'
# my settings
model = "o3"

# a comment about notes
[mcp_servers.notes]
url = "http://127.0.0.1:7000/mcp"

[mcp_servers.linggen]
url = "http://192.168.1.5:9527/mcp"

[profiles.fast]
model = "o4-mini" # inline comment
TOML

# The session-start hook, as Codex runs it (no Linggen answering, nothing
# installed).
run_hook() {
  printf '{"session_id":"s1","cwd":"%s"}' "$tmp" \
    | env CLAUDE_PLUGIN_ROOT="$HOME/.codex/plugins/linggen" LINGGEN_PORT=1 \
      LINGGEN_NO_ENGINE_INSTALL=1 bash "$plugin/hooks/autostart.sh" >/dev/null 2>&1
}

run_hook
diff -u "$tmp/want" "$config" >&2 || fail "the retired memory server is not removed alone"

# A second start changes nothing, not even the file's time.
before="$(ls -l -T "$config" 2>/dev/null || ls -l --time-style=full-iso "$config")"
sleep 1
run_hook
cmp -s "$tmp/want" "$config" || fail "a second start changed the file"
after="$(ls -l -T "$config" 2>/dev/null || ls -l --time-style=full-iso "$config")"
[ "$before" = "$after" ] || fail "a second start rewrote the file"

# A `memory` server of the person's own (another url) stays, byte for byte.
cat >"$config" <<'TOML'
[mcp_servers.memory]
url = "http://127.0.0.1:8000/mcp"

[mcp_servers.memory.env]
KEY = "v"
TOML
cp "$config" "$tmp/own"
run_hook
cmp -s "$tmp/own" "$config" || fail "a memory server of the person's own was touched"

printf 'ok: Codex start removes only the retired memory server\n'
