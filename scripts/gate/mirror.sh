# Release gate — build the release mirror (doc/release-checklist.md § Release
# gate, doc/cli.md § Release override). Sourced by gate.sh.
#
#   build_mirror local                 engine/ling-mem/app from local builds
#   build_mirror draft "<specs>"       gh release download; specs like
#                                      "engine=1.8.3 mem=v1.9.0 app=linggen-v0.3.4";
#                                      a repo left out serves its latest release
#
# The shape follows the engine under test: from 2.3.0 memory is inside `ling`
# (ONE_BINARY=1) — no ling-mem release is part of the train, and the mirror
# serves the published ling-mem only because install.sh and the plugin
# installers still fetch it (the engine turns it into its stand-in). Before
# 2.3.0 ling-mem is its own release, gated with the engine. GATE_ONE_BINARY=1|0
# overrides the version (a local build carries the last release's number until
# the release stamps it).
#
# Three trees under $RUN/mirror, each a complete LINGGEN_RELEASE_BASE:
#   good/     the release under test
#   relabel/  same bytes, engine manifest version "<v>-gate" — lets `ling
#             update` run when the installed binary already is <v>
#   bad/      relabel with every sha256 wrong — every updater must refuse it
# and $RUN/mirror/expect.env: the versions and binary sha256s to check for.

ENGINE_REPO=linggen/linggen
MEM_REPO=linggen/linggen-memory
APP_REPO=linggen/linggen-releases
ZERO_SHA=0000000000000000000000000000000000000000000000000000000000000000

sha() { shasum -a 256 "$1" | awk '{print $1}'; }

ONE_BINARY_FROM=2.3.0
version_at_least() { [ "$(printf '%s\n%s\n' "$2" "$1" | sort -V | head -1)" = "$2" ]; } # v min

# ONE_BINARY from the engine under test (LING_VERSION), unless overridden.
decide_shape() {
  case "${GATE_ONE_BINARY:-}" in
    1) ONE_BINARY=1; record INFO "mirror: shape" "one program (GATE_ONE_BINARY=1; engine v$LING_VERSION)"; return ;;
    0) ONE_BINARY=0; record INFO "mirror: shape" "engine + ling-mem (GATE_ONE_BINARY=0; engine v$LING_VERSION)"; return ;;
  esac
  if version_at_least "${LING_VERSION%%-*}" "$ONE_BINARY_FROM"; then ONE_BINARY=1
    record INFO "mirror: shape" "one program — engine v$LING_VERSION hosts memory (≥ $ONE_BINARY_FROM)"
  else ONE_BINARY=0
    record INFO "mirror: shape" "engine + ling-mem — engine v$LING_VERSION is before $ONE_BINARY_FROM"
  fi
}

# One program: the published ling-mem (or the release named), served for the
# installers that still fetch it; never part of the train. Soft: without it
# install.sh says it could not install ling-mem and goes on.
mirror_published_mem() { # good [tag]
  local good="$1" tag="${2:-}"
  tag="${tag:-$(latest_tag $MEM_REPO 2>/dev/null)}"
  mkdir -p "$good/$MEM_REPO"
  if [ -n "$tag" ] && gh release download "$tag" --repo $MEM_REPO --dir "$good/$MEM_REPO" --clobber \
       --pattern 'ling-mem-macos-aarch64.tar.gz*' >/dev/null 2>&1; then
    gh release view "$tag" --repo $MEM_REPO --json tagName,assets \
      --jq '{tag_name: .tagName, assets: [.assets[] | {name}]}' >"$good/$MEM_REPO/release.json"
    MEM_VERSION="${tag#v}"
    MEM_SHA="$(tar_member_sha "$good/$MEM_REPO/ling-mem-macos-aarch64.tar.gz" ling-mem)"
    record INFO "mirror: ling-mem" "published $tag — for install.sh and the plugins only, not part of this train"
  else
    MEM_VERSION="" MEM_SHA=""
    record INFO "mirror: ling-mem" "none served (${tag:-no published release found}) — install.sh skips it, the engine writes its stand-in"
  fi
}

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
  local good="$1" ling mem app ver
  ling="${GATE_LING_BIN:-$REPO/target/release/ling}"
  mem="${GATE_MEM_BIN:-$WS/linggen-memory/target/release/ling-mem}"
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
  decide_shape

  if [ "$ONE_BINARY" = 1 ]; then
    mirror_published_mem "$good"
    pack_plugin "$good"
    [ -f "$good/$MEM_REPO/release.json" ] || write_release_json "$good/$MEM_REPO" "v0.0.0"
  else
    [ -x "$mem" ] || die "mirror: ling-mem" "no local build at $mem (GATE_MEM_BIN)"
    ver="$("$mem" --version | awk '{print $2}')"
    mkdir -p "$good/$MEM_REPO"
    MEM_SHA="$(pack_binary "$mem" ling-mem "$good/$MEM_REPO/ling-mem-macos-aarch64.tar.gz")"
    MEM_VERSION="$ver"
    (cd "$good/$MEM_REPO" && shasum -a 256 ling-mem-macos-aarch64.tar.gz >ling-mem-macos-aarch64.tar.gz.sha256)
    pack_plugin "$good"
    write_release_json "$good/$MEM_REPO" "v$ver"
    record INFO "mirror: ling-mem" "local $ver, $mem, built $(stat -f %Sm -t '%F %R' "$mem"), $(signer "$mem")"
  fi

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

# The plugin marketplace bundle install-plugin.sh installs from (linggen-
# memory's release asset), packed from the working tree.
pack_plugin() { # good
  "$WS/linggen-memory/scripts/package-plugin.sh" "$1/$MEM_REPO/linggen-plugin.tar.gz" >/dev/null \
    || die "mirror: plugin bundle" "linggen-memory/scripts/package-plugin.sh failed"
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
  local good="$1" specs="$2" engine mem app kv
  for kv in $specs; do
    case "$kv" in
      engine=*) engine="${kv#engine=}" ;;
      mem=*) mem="${kv#mem=}" ;;
      app=*) app="${kv#app=}" ;;
      *) die "mirror" "unknown draft spec '$kv' (engine=… mem=… app=…)" ;;
    esac
  done
  engine="${engine:-$(latest_tag $ENGINE_REPO)}"
  local mem_named="${mem:-}"
  mem="${mem:-$(latest_tag $MEM_REPO)}"
  # No app= draft: the app is cut after the engine ships, so the mirror
  # serves the published app — not this train's, and not gated as one.
  APP_IN_TRAIN=1; [ -n "$app" ] || APP_IN_TRAIN=0
  app="${app:-$(latest_tag $APP_REPO)}"

  draft_repo "$good" $ENGINE_REPO "$engine" manifest.json 'ling-macos-aarch64.tar.gz'
  LING_VERSION="$(jq -r .version "$good/$ENGINE_REPO/manifest.json")"
  LING_SHA="$(tar_member_sha "$good/$ENGINE_REPO/ling-macos-aarch64.tar.gz" ling)"
  record INFO "mirror: engine" "release $engine (manifest $LING_VERSION)"
  decide_shape

  if [ "$ONE_BINARY" = 1 ]; then
    mirror_published_mem "$good" "$mem_named"
  else
    draft_repo "$good" $MEM_REPO "$mem" 'ling-mem-macos-aarch64.tar.gz*'
    MEM_VERSION="${mem#v}"
    MEM_SHA="$(tar_member_sha "$good/$MEM_REPO/ling-mem-macos-aarch64.tar.gz" ling-mem)"
    record INFO "mirror: ling-mem" "release $mem"
  fi
  if gh release download "$mem" --repo $MEM_REPO --dir "$good/$MEM_REPO" --clobber --pattern 'linggen-plugin.tar.gz*' 2>/dev/null; then
    record INFO "mirror: plugin bundle" "release $mem"
  else
    pack_plugin "$good"
    record GAP "mirror: plugin bundle" "release $mem has no linggen-plugin.tar.gz — packed from the working tree"
  fi

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
  for f in "$dst"/$MEM_REPO/*.sha256 "$dst"/$APP_REPO/*.sha256; do
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
  local pub="$WS/linggensite/public" f
  for f in install.sh install-app.sh install-shared-memory.sh install-plugin.sh; do
    [ -f "$pub/$f" ] || die "mirror: installers" "missing $pub/$f"
    cp "$pub/$f" "$good/$f"
  done
  cp "$WS/linggen-memory/plugins/linggen/scripts/install-bin.sh" "$good/install-bin.sh"
  derive_tree "$good" "$root/relabel" "-gate" 0
  derive_tree "$good" "$root/bad" "-gate" 1
  cat >"$root/expect.env" <<EOF
LING_VERSION=$LING_VERSION
LING_SHA=$LING_SHA
MEM_VERSION=$MEM_VERSION
MEM_SHA=$MEM_SHA
ONE_BINARY=${ONE_BINARY:-0}
APP_VERSION=$APP_VERSION
APP_IN_TRAIN=${APP_IN_TRAIN:-1}
EOF
  if [ "${ONE_BINARY:-0}" = 1 ]; then
    record PASS "mirror built" "ling $LING_VERSION (memory inside), app ${APP_VERSION:-none}; published ling-mem ${MEM_VERSION:-none} for the installers"
  else
    record PASS "mirror built" "ling $LING_VERSION, ling-mem $MEM_VERSION, app ${APP_VERSION:-none}"
  fi
}
