#!/usr/bin/env bash
# Release gate — a clean Mac (Tart VM) installs and upgrades the release
# before it is published. doc/test-design.md § 4, doc/release-checklist.md.
#
#   just release-gate --local                       local builds (target/release)
#   just release-gate --draft "engine=1.8.3 mem=v1.9.0 app=linggen-v0.3.4"
#   just release-gate --public                      after publishing: the VM installs
#                                                   from linggen.dev + GitHub Latest,
#                                                   no mirror (first install only)
#
#   --first-only / --upgrade-only   run one half
#   --rebuild-prev                  remake linggen-prev from the published releases
#   --save-prev                     on green, the upgraded VM becomes linggen-prev
#                                   (default for --draft; --local keeps the baseline)
#   --prev <vm>                     also upgrade this frozen old-user VM (repeatable,
#                                   or GATE_PREV_VMS="a b"); never rolled forward
#   --skip-host-checks              don't run check.sh all + e2e first (a GAP)
#   --keep                          leave the scratch VMs (stopped) for a look
#
# Exit 0 only when no check FAILs; the table at the end lists every check.
# Local builds can be overridden: GATE_LING_BIN, GATE_MEM_BIN, GATE_APP.
# The VM image: GATE_IMAGE, else linggen-gate-base if present, else vanilla.

set -uo pipefail
. "$(dirname "$0")/lib.sh"
. "$GATE_SRC/mirror.sh"

usage() { echo "release-gate: $*" >&2; sed -n '2,22p' "$0" >&2; exit 2; }
needs_arg() { [ $# -ge 2 ] && [ -n "$2" ] && [ "${2#--}" = "$2" ] || usage "$1 needs a value"; }

SOURCE=local SPECS="" FIRST=1 UPGRADE=1 REBUILD_PREV=0 SAVE_PREV="" KEEP=0 HOST_CHECKS=1
read -ra EXTRA_PREVS <<<"${GATE_PREV_VMS:-}"
while [ $# -gt 0 ]; do
  case "$1" in
    --local) SOURCE=local; shift ;;
    --draft) needs_arg "$@"; SOURCE=draft; SPECS="$2"; shift 2 ;;
    --prev) needs_arg "$@"; EXTRA_PREVS+=("$2"); shift 2 ;;
    --skip-host-checks) HOST_CHECKS=0; shift ;;
    --public) SOURCE=public; UPGRADE=0; shift ;;
    --first-only) UPGRADE=0; shift ;;
    --upgrade-only) FIRST=0; shift ;;
    --rebuild-prev) REBUILD_PREV=1; shift ;;
    --save-prev) SAVE_PREV=1; shift ;;
    --keep) KEEP=1; shift ;;
    -h|--help) sed -n '2,22p' "$0"; exit 0 ;;
    *) usage "unknown flag $1" ;;
  esac
done
[ -n "$SAVE_PREV" ] || { [ "$SOURCE" = draft ] && SAVE_PREV=1 || SAVE_PREV=0; }

RUN="$GATE_HOME/runs/$(date +%Y%m%d-%H%M%S)"
mkdir -p "$RUN"
RESULTS="$RUN/results.tsv"; : >"$RESULTS"
trap cleanup EXIT
trap 'exit 130' INT TERM
[ "$KEEP" = 1 ] && cleanup() { local rc=$? p; vm_untunnel; for vm in "${SCRATCH_VMS[@]+"${SCRATCH_VMS[@]}"}"; do tart stop "$vm" >/dev/null 2>&1; done; for p in "${PIDS[@]+"${PIDS[@]}"}"; do kill "$p" 2>/dev/null; done; exit "$rc"; }

# ── Preflight ──────────────────────────────────────────────────────────────
say "Preflight ($RUN)"
for tool in tart jq curl python3 ssh scp; do
  command -v "$tool" >/dev/null || die "preflight: $tool" "not installed"
done
[ "$SOURCE" != local ] && { command -v gh >/dev/null || die "preflight: gh" "not installed"; }
free_gb="$(df -g "$HOME" | awk 'NR==2{print $4}')"
[ "$free_gb" -ge 8 ] && record PASS "preflight: disk" "${free_gb} GB free" \
  || record WARN "preflight: disk" "only ${free_gb} GB free — a VM run writes 2-4 GB"
# BASE_IMAGE (lib.sh): a local VM such as linggen-gate-base, or an OCI image
# pulled once.
if [ -z "$(vm_state "$BASE_IMAGE")" ]; then
  case "$BASE_IMAGE" in */*) ;; *) die "preflight: image" "no local VM $BASE_IMAGE (tart list)" ;; esac
  say "Pulling $BASE_IMAGE (~25 GB, once)"
  tart pull "$BASE_IMAGE" || die "preflight: image" "tart pull $BASE_IMAGE failed"
fi
[ "$BASE_IMAGE" = "$VANILLA_IMAGE" ] \
  && record PASS "preflight: image" "$BASE_IMAGE (no $GATE_BASE: a quarantined Developer ID launch is a GAP)" \
  || record PASS "preflight: image" "$BASE_IMAGE"
for vm in "${EXTRA_PREVS[@]+"${EXTRA_PREVS[@]}"}"; do
  [ -n "$(vm_state "$vm")" ] || die "preflight: --prev $vm" "no such VM (freeze one: tart clone $PREV_VM $vm)"
done
ensure_key

# ── Host suites, on the commit being released ──────────────────────────────
ENGINE_HEAD="$(git -C "$REPO" rev-parse HEAD)"
dirty="$(git -C "$REPO" status --porcelain --untracked-files=no | wc -l | tr -d ' ')"
record INFO "host: engine HEAD" "$ENGINE_HEAD$([ "$dirty" = 0 ] || echo " + $dirty changed files")"
if [ "$SOURCE" = draft ]; then
  [ "$dirty" = 0 ] || record WARN "host: clean tree" "$dirty changed files — the host suites test them, not the draft's commit"
  # A draft's tag is cut at publish, at its target: a sha, or a branch's
  # remote head. Its binary was built from the commit the target names.
  tag="$(sed -n 's/.*engine=\([^ ]*\).*/\1/p' <<<"$SPECS")"
  if [ -n "$tag" ]; then
    target="$(gh release view "$tag" --repo linggen/linggen --json targetCommitish --jq .targetCommitish 2>/dev/null)"
    at="$(git -C "$REPO" ls-remote origin "refs/tags/$tag^{}" "refs/tags/$tag" 2>/dev/null | awk 'END{print $1}')"
    if [ -n "$at" ]; then how="tag $tag"
    elif [[ "$target" =~ ^[0-9a-f]{40}$ ]]; then at="$target" how="draft target"
    elif [ -n "$target" ]; then at="$(git -C "$REPO" ls-remote origin "refs/heads/$target" | cut -f1)" how="origin/$target (the draft's target)"
    fi
    if [ -z "$at" ]; then record WARN "host: tested commit is the draft's" "can't tell what $tag is cut from (target '${target:-?}')"
    elif [ "$at" = "$ENGINE_HEAD" ]; then record PASS "host: tested commit is the draft's" "$how = HEAD ${ENGINE_HEAD:0:12}"
    elif [ "$how" = "draft target" ]; then record FAIL "host: tested commit is the draft's" "$how ${at:0:12} ≠ HEAD ${ENGINE_HEAD:0:12}"
    else record WARN "host: tested commit is the draft's" "$how ${at:0:12} ≠ HEAD ${ENGINE_HEAD:0:12}"; fi
  fi
fi
if [ "$SOURCE" = public ]; then
  record INFO "host: check.sh all + e2e" "not run — --public gates what is already published"
elif [ "$HOST_CHECKS" = 0 ]; then
  record GAP "host: check.sh all + e2e" "skipped (--skip-host-checks)"
else
  for suite in all e2e; do
    say "Host suites: check.sh $suite"
    (cd "$REPO" && ./scripts/check.sh "$suite") >"$RUN/host-check-$suite.log" 2>&1
    if [ $? = 0 ]; then record PASS "host: check.sh $suite" "${ENGINE_HEAD:0:12}"
    else record FAIL "host: check.sh $suite" "$(grep -m3 '^FAILED' "$RUN/host-check-$suite.log" | tr '\n' ' ') (see $RUN/host-check-$suite.log)"; fi
  done
  # A red host suite fails the gate: no VM time spent on it.
  [ "$(awk -F'\t' '$3=="FAIL"' "$RESULTS" | wc -l | tr -d ' ')" = 0 ] || { PHASE=summary; summary; exit 1; }
fi

# ── Mirror + fake model + what the VM gets ─────────────────────────────────
say "Mirror ($SOURCE)"
build_mirror "$SOURCE" "$SPECS"
VMFILES="$RUN/vmfiles"
stage_vmfiles() {
  local f
  mkdir -p "$VMFILES/marketplace"
  cp "$GATE_SRC"/vm/*.sh "$GATE_SRC/vm/gate.runtime.toml" "$RUN/mirror/expect.env" "$VMFILES/"
  # The plugin bundle of this release (what install-plugin.sh installs), to
  # compare against and as linggen-prev's local marketplace.
  tar -xzf "$RUN/mirror/good/linggen/linggen-memory/linggen-plugin.tar.gz" -C "$VMFILES/marketplace"
  # Codex has no unattended installer without node; hand in this Mac's CLI.
  f="$(realpath "$(command -v codex 2>/dev/null)" 2>/dev/null || true)"
  [ -n "$f" ] && file "$f" | grep -q 'Mach-O.*arm64' && cp "$f" "$VMFILES/codex"
  # The fixtures home, rendered for the VM's admin user.
  cp -R "$REPO/tests/fixtures/home" "$VMFILES/home"
  cp "$REPO/tests/fixtures/memory/alex.json" "$VMFILES/alex.json"
  grep -rlE '\{\{[A-Z_]+\}\}' "$VMFILES/home" "$VMFILES/alex.json" | while read -r f; do
    perl -pi -e 's#\{\{LINGGEN_HOME\}\}#/Users/admin/.linggen#g; s#\{\{ENGINE_PORT\}\}#9527#g;
                 s#\{\{MEM_URL\}\}#http://127.0.0.1:9528#g; s#\{\{MODEL_URL\}\}#'"$VM_MODEL"'#g;
                 s#\{\{WORK\}\}#/Users/admin/work#g' "$f"
  done
}
stage_vmfiles
start_mirror "$RUN/mirror"
start_model
record PASS "servers" "mirror 127.0.0.1:$MIRROR_PORT, llmposter 127.0.0.1:$MODEL_PORT (VM: $VM_MIRROR, $VM_MODEL)"
VMENV=(MIRROR="$VM_MIRROR" MODEL="$VM_MODEL" GATE_SOURCE="$SOURCE" DREAM_SHA="$(shasum -a 256 "$REPO/missions/dream/mission.md" | awk '{print $1}')")

# The UI in a real browser over the tunnel (Playwright from tests/e2e).
ui_check() { # vm label
  local e2e="$REPO/tests/e2e" out
  vm_ssh "$1" 'cd ~/gate && MIRROR=x && . ./common.sh && start_engine 60' >/dev/null 2>&1
  if [ ! -d "$e2e/node_modules/@playwright/test" ]; then
    record GAP "$2: UI in a browser" "tests/e2e has no Playwright installed (npm ci there) — curl check only"
    return
  fi
  out="$(E2E_DIR="$e2e" node "$GATE_SRC/ui-check.mjs" "http://127.0.0.1:$UI_PORT/" "$RUN/$2-ui.png" 2>&1)"
  if [ $? = 0 ]; then record PASS "$2: UI renders in Chromium" "$(printf '%s' "$out" | tail -1 | cut -c1-160)"
  else record FAIL "$2: UI renders in Chromium" "$(printf '%s' "$out" | tail -1 | cut -c1-200)"; fi
  vm_ssh "$1" 'export PATH=$HOME/.local/bin:$PATH; ling stop' >/dev/null 2>&1
}

phase_fails() { awk -F'\t' -v p="$1" '$1==p && $3=="FAIL"' "$RESULTS" | wc -l | tr -d ' '; }

# ── First install ──────────────────────────────────────────────────────────
if [ "$FIRST" = 1 ]; then
  PHASE=first-install
  say "First install — fresh $BASE_IMAGE"
  vm_clone "$BASE_IMAGE" gate-first
  vm_boot gate-first
  record INFO "vm" "$(vm_ssh gate-first 'sw_vers -productVersion') at $(vm_ip gate-first)"
  vm_tunnels gate-first
  vm_run gate-first first-install.sh "${VMENV[@]}"
  ui_check gate-first first-install
  vm_shutdown gate-first
  [ "$KEEP" = 1 ] || { tart delete gate-first >/dev/null 2>&1; SCRATCH_VMS=("${SCRATCH_VMS[@]/gate-first}"); }
fi

# ── Upgrade ────────────────────────────────────────────────────────────────
if [ "$UPGRADE" = 1 ]; then
  if [ "$REBUILD_PREV" = 1 ] || [ -z "$(vm_state "$PREV_VM")" ]; then
    PHASE=prev
    say "Building $PREV_VM — published releases + fixtures home"
    vm_clone "$BASE_IMAGE" gate-prev-build
    vm_boot gate-prev-build
    vm_tunnels gate-prev-build
    vm_run gate-prev-build prev-setup.sh "${VMENV[@]}"
    vm_shutdown gate-prev-build
    [ "$(phase_fails prev)" = 0 ] || die "prev: $PREV_VM" "setup failed — the old baseline is not saved"
    tart delete "$PREV_VM" >/dev/null 2>&1
    tart rename gate-prev-build "$PREV_VM" || die "prev: $PREV_VM" "tart rename failed"
    SCRATCH_VMS=("${SCRATCH_VMS[@]/gate-prev-build}")
    record PASS "prev: $PREV_VM saved" ""
  fi
  upgrade_from() { # baseline scratch-vm phase
    PHASE="$3"
    say "Upgrade — clone of $1"
    vm_clone "$1" "$2"
    vm_boot "$2"
    vm_tunnels "$2"
    vm_run "$2" upgrade.sh "${VMENV[@]}"
    ui_check "$2" "$3"
    vm_shutdown "$2"
  }
  upgrade_from "$PREV_VM" gate-upgrade upgrade
  # Frozen older baselines (--prev): upgraded too, never rolled forward.
  n=0
  for vm in "${EXTRA_PREVS[@]+"${EXTRA_PREVS[@]}"}"; do
    [ "$vm" = "$PREV_VM" ] && continue
    n=$((n + 1))
    upgrade_from "$vm" "gate-upgrade-$n" "upgrade:$vm"
    [ "$KEEP" = 1 ] || { tart delete "gate-upgrade-$n" >/dev/null 2>&1; SCRATCH_VMS=("${SCRATCH_VMS[@]/gate-upgrade-$n}"); }
  done
  PHASE=upgrade
  if [ "$(awk -F'\t' '$3=="FAIL"' "$RESULTS" | wc -l | tr -d ' ')" = 0 ] && [ "$SAVE_PREV" = 1 ]; then
    tart delete "$PREV_VM" >/dev/null 2>&1
    tart rename gate-upgrade "$PREV_VM" && SCRATCH_VMS=("${SCRATCH_VMS[@]/gate-upgrade}") \
      && record INFO "next $PREV_VM" "the upgraded VM is the new old-user baseline"
  fi
fi

PHASE=summary
summary
