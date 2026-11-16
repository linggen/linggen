# Release gate — build the release mirror (doc/release-checklist.md § Release
# gate, doc/cli.md § Release override). Sourced by gate.sh.
#
#   build_mirror local                 engine/app from local builds
#   build_mirror draft "<specs>"       gh release download; specs like
#                                      "engine=2.3.0 app=linggen-v2.3.0";
#                                      a repo left out serves its latest release
#
# Memory is inside `ling`: the train is the engine (with the Claude Code /
# Codex plugin bundle, linggen-plugin.tar.gz, on the same release) and the app.
#
# Three trees under $RUN/mirror, each a complete LINGGEN_RELEASE_BASE:
#   good/     the release under test
#   relabel/  same bytes, engine manifest version "<v>-gate" — lets `ling
#             update` run when the installed binary already is <v>
#   bad/      relabel with every sha256 wrong — every updater must refuse it
# and $RUN/mirror/expect.env: the versions and binary sha256s to check for.
#
# The installers are not release assets: GATE_SITE_PUBLIC is a folder like
# linggensite/public (default: the sibling checkout's) — point it at a
# worktree of the release-day site branch.

ENGINE_REPO=linggen/linggen
APP_REPO=linggen/linggen-releases
PLUGIN_ASSET=linggen-plugin.tar.gz
ZERO_SHA=0000000000000000000000000000000000000000000000000000000000000000
SITE_PUBLIC="${GATE_SITE_PUBLIC:-$WS/linggensite/public}"

sha() { shasum -a 256 "$1" | awk '{print $1}'; }

# {"tag_name", "assets": [{"name"}]} for the files in a repo dir.
write_release_json() { # dir tag
  local names
  names="$(cd "$1" && ls | grep -v '^release.json$' | jq -R '{name: .}' | jq -s .)"
  jq -n --arg t "$2" --argjson a "$names" '{tag_name: $t, assets: $a}' >"$1/release.json"
}

# A Developer ID signature (scripts/sign-mac.sh) is kept as built; anything
# else is ad-hoc signed, as a dev build is.
signer() { developer_id "$1" && echo "Developer ID" || echo "ad-hoc"; }
developer_id() { local d; d="$(codesign -dvv "$1" 2>&1)"; grep -q '^Authority=Developer ID Application' <<<"$d"; }

pack_binary() { # src name dest-tarball
  local stage; stage="$(mktemp -d "$RUN/pack.XXXX")"
  cp "$1" "$stage/$2"
  developer_id "$stage/$2" || codesign --force --sign - "$stage/$2" 2>/dev/null
  tar -C "$stage" -czf "$3" "$2"
  echo "$(sha "$stage/$2")"
  rm -rf "$stage"
}

mirror_local() {
  local good="$1" ling app ver
  ling="${GATE_LING_BIN:-$REPO/target/release/ling}"
  app="${GATE_APP:-$WS/linggen-app/shell/target/aarch64-apple-darwin/release/bundle/macos/Linggen.app}"
  [ -x "$ling" ] || die "mirror: engine" "no local build at $ling (cargo build --release)"

  ver="$("$ling" --version | awk '{print $2}')"
  mkdir -p "$good/$ENGINE_REPO"
  LING_SHA="$(pack_binary "$ling" ling "$good/$ENGINE_REPO/ling-macos-aarch64.tar.gz")"
  LING_VERSION="$ver"
  jq -n --arg v "$ver" --arg s "$(sha "$good/$ENGINE_REPO/ling-macos-aarch64.tar.gz")" \
    '{version: $v, assets: [{name: "ling-macos-aarch64",
      url: ("https://github.com/linggen/linggen/releases/download/" + $v + "/ling-macos-aarch64.tar.gz"),
      sha256: $s}]}' >"$good/$ENGINE_REPO/manifest.json"
  record INFO "mirror: engine" "local $ver, $ling, built $(stat -f %Sm -t '%F %R' "$ling"), $(signer "$ling")"
  pack_plugin "$good"
  write_release_json "$good/$ENGINE_REPO" "$ver"

  mkdir -p "$good/$APP_REPO"
  if [ -d "$app" ]; then
    local stage asset
    ver="$(plutil -extract CFBundleShortVersionString raw "$app/Contents/Info.plist")"
    asset="linggen-$ver-darwin-arm64.tar.gz"
    stage="$(mktemp -d "$RUN/pack.XXXX")"
    ditto "$app" "$stage/Linggen.app"
    developer_id "$stage/Linggen.app" || codesign --force --deep --sign - "$stage/Linggen.app" 2>/dev/null
    tar -C "$stage" -czf "$good/$APP_REPO/$asset" Linggen.app
    rm -rf "$stage"
    (cd "$good/$APP_REPO" && shasum -a 256 "$asset" >"$asset.sha256")
    write_release_json "$good/$APP_REPO" "linggen-v$ver"
    APP_VERSION="$ver"; APP_IN_TRAIN=1
    record INFO "mirror: app" "local $ver, built $(stat -f %Sm -t '%F %R' "$app"), bundled engine $("$app/Contents/MacOS/ling" --version | awk '{print $2}'), $(signer "$app")"
  else
    APP_VERSION=""
    record GAP "mirror: app" "no local build at $app (linggen-app: ./scripts/build.sh linggen)"
  fi
}

# The plugin bundle install-plugin.sh installs (an engine release asset),
# packed from this checkout as release.sh packs it.
pack_plugin() { # good
  local out="$1/$ENGINE_REPO/$PLUGIN_ASSET"
  "$REPO/scripts/plugin/package-plugin.sh" "$out" >/dev/null \
    || die "mirror: plugin bundle" "$REPO/scripts/plugin/package-plugin.sh failed"
  record INFO "mirror: plugin bundle" "packed from $REPO ($(git -C "$REPO" rev-parse --abbrev-ref HEAD 2>/dev/null) $(git -C "$REPO" rev-parse --short HEAD 2>/dev/null)), sha256 $(sha "$out" | cut -c1-12)"
}

latest_tag() { gh release view --repo "$1" --json tagName --jq .tagName; }

draft_repo() { # good repo tag patterns…
  local good="$1" repo="$2" tag="$3"; shift 3
  local args=() p
  for p in "$@"; do args+=(--pattern "$p"); done
  mkdir -p "$good/$repo"
  gh release download "$tag" --repo "$repo" --dir "$good/$repo" --clobber "${args[@]}" \
    || die "mirror: $repo" "gh release download $tag failed"
  gh release view "$tag" --repo "$repo" --json tagName,assets \
    --jq '{tag_name: .tagName, assets: [.assets[] | {name}]}' >"$good/$repo/release.json"
}

tar_member_sha() { # tarball member
  local d; d="$(mktemp -d "$RUN/unpack.XXXX")"
  tar -xzf "$1" -C "$d" "$2" && sha "$d/$2"
  rm -rf "$d"
}

mirror_draft() { # good specs
  local good="$1" specs="$2" engine app kv
  for kv in $specs; do
    case "$kv" in
      engine=*) engine="${kv#engine=}" ;;
      app=*) app="${kv#app=}" ;;
      *) die "mirror" "unknown draft spec '$kv' (engine=… app=…)" ;;
    esac
  done
  engine="${engine:-$(latest_tag $ENGINE_REPO)}"
  # No app= draft: the app is cut after the engine ships, so the mirror
  # serves the published app — not this train's, and not gated as one.
  APP_IN_TRAIN=1; [ -n "$app" ] || APP_IN_TRAIN=0
  app="${app:-$(latest_tag $APP_REPO)}"

  # The plugin bundle rides on the engine release: one without it fails here.
  draft_repo "$good" $ENGINE_REPO "$engine" manifest.json 'ling-macos-aarch64.tar.gz' "$PLUGIN_ASSET*"
  [ -f "$good/$ENGINE_REPO/$PLUGIN_ASSET" ] && [ -f "$good/$ENGINE_REPO/$PLUGIN_ASSET.sha256" ] \
    || die "mirror: plugin bundle" "release $engine carries no $PLUGIN_ASSET + .sha256 (scripts/release.sh attaches them)"
  LING_VERSION="$(jq -r .version "$good/$ENGINE_REPO/manifest.json")"
  LING_SHA="$(tar_member_sha "$good/$ENGINE_REPO/ling-macos-aarch64.tar.gz" ling)"
  record INFO "mirror: engine" "release $engine (manifest $LING_VERSION)"
  record INFO "mirror: plugin bundle" "release $engine, sha256 $(sha "$good/$ENGINE_REPO/$PLUGIN_ASSET" | cut -c1-12)"

  draft_repo "$good" $APP_REPO "$app" 'linggen-*-darwin-arm64.tar.gz*'
  APP_VERSION="${app#linggen-v}"
  record INFO "mirror: app" "release $app$([ "$APP_IN_TRAIN" = 1 ] || echo ' (published; not in this train)')"
}

# relabel/ and bad/: copies of the small files, links to the tarballs.
derive_tree() { # good dst version-suffix bad(0|1)
  local good="$1" dst="$2" f
  mkdir -p "$dst"
  (cd "$good" && find . -type d) | while read -r d; do mkdir -p "$dst/$d"; done
  (cd "$good" && find . -type f) | while read -r f; do
    case "$f" in
      *.tar.gz) ln -s "$good/$f" "$dst/$f" ;;
      *) cp "$good/$f" "$dst/$f" ;;
    esac
  done
  jq --arg sfx "$3" '.version += $sfx' "$good/$ENGINE_REPO/manifest.json" >"$dst/$ENGINE_REPO/manifest.json"
  [ "$4" = 1 ] || return 0
  jq --arg z "$ZERO_SHA" '.assets |= map(.sha256 = $z)' "$dst/$ENGINE_REPO/manifest.json" >"$dst/m.tmp" \
    && mv "$dst/m.tmp" "$dst/$ENGINE_REPO/manifest.json"
  for f in "$dst"/$ENGINE_REPO/*.sha256 "$dst"/$APP_REPO/*.sha256; do
    [ -f "$f" ] && printf '%s  %s\n' "$ZERO_SHA" "$(basename "${f%.sha256}")" >"$f"
  done
  return 0
}

build_mirror() { # local | draft "<specs>"
  local root="$RUN/mirror" good
  good="$root/good"
  rm -rf "$root"; mkdir -p "$good"
  case "$1" in
    local) mirror_local "$good" ;;
    draft) mirror_draft "$good" "$2" ;;
    # The published Latest releases: what to expect; the VM itself installs
    # from linggen.dev + GitHub (vm/common.sh SITE, asset_url).
    public) mirror_draft "$good" "" ;;
  esac
  local pub="$SITE_PUBLIC" f
  for f in install.sh install-app.sh install-plugin.sh; do
    [ -f "$pub/$f" ] || die "mirror: installers" "missing $pub/$f"
    cp "$pub/$f" "$good/$f"
  done
  record INFO "mirror: installers" "$pub ($(git -C "$pub" rev-parse --abbrev-ref HEAD 2>/dev/null) $(git -C "$pub" rev-parse --short HEAD 2>/dev/null))"
  derive_tree "$good" "$root/relabel" "-gate" 0
  derive_tree "$good" "$root/bad" "-gate" 1
  cat >"$root/expect.env" <<EOF
LING_VERSION=$LING_VERSION
LING_SHA=$LING_SHA
APP_VERSION=$APP_VERSION
APP_IN_TRAIN=${APP_IN_TRAIN:-1}
EOF
  record PASS "mirror built" "ling $LING_VERSION (memory inside), plugin bundle, app ${APP_VERSION:-none}"
}
