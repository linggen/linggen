# Release gate — host-side helpers (doc/test-design.md § 4). Sourced by gate.sh.
#
# Everything the gate installs goes into Tart VMs; on the host it only runs
# two loopback servers (the release mirror and llmposter) that reach the VM
# through SSH reverse tunnels, so nothing listens on the LAN and the host's
# own ~/.linggen, 9527/9528 and install dirs are never touched.

GATE_SRC="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
REPO="$(cd "$GATE_SRC/../.." && pwd)"
WS="$(cd "$REPO/.." && pwd)"

GATE_HOME="${LINGGEN_GATE_HOME:-$HOME/.cache/linggen-gate}"
VANILLA_IMAGE=ghcr.io/cirruslabs/macos-tahoe-vanilla:latest
# The VM image: GATE_IMAGE, else the local linggen-gate-base (vanilla switched
# by hand to "App Store & Known Developers", doc/release-checklist.md), else
# vanilla — where a quarantined Developer ID launch is a GAP.
GATE_BASE=linggen-gate-base
if [ -n "${GATE_IMAGE:-}" ]; then BASE_IMAGE="$GATE_IMAGE"
elif tart list --format json 2>/dev/null | jq -e --arg n "$GATE_BASE" '.[] | select(.Name == $n)' >/dev/null 2>&1; then BASE_IMAGE="$GATE_BASE"
else BASE_IMAGE="$VANILLA_IMAGE"; fi
PREV_VM="${GATE_PREV_VM:-linggen-prev}"
SSH_KEY="$GATE_HOME/id_ed25519"
LLMPOSTER="${LLMPOSTER:-$GATE_HOME/bin/llmposter}"

# In the VM, the mirror and the fake model are on loopback (reverse tunnels).
VM_MIRROR_PORT=18765
VM_MODEL_PORT=18766
VM_MIRROR="http://127.0.0.1:$VM_MIRROR_PORT"
VM_MODEL="http://127.0.0.1:$VM_MODEL_PORT"

# ── Results ──────────────────────────────────────────────────────────────────
# One TSV row per check: phase, check, status (PASS FAIL WARN GAP INFO), detail.
# FAIL fails the gate; WARN/GAP/INFO are read by a person.

RESULTS=""   # set by gate.sh to $RUN/results.tsv
PHASE="setup"

record() { # status check [detail]
  local status="$1" check="$2" detail="${3:-}"
  printf '%s\t%s\t%s\t%s\n' "$PHASE" "$check" "$status" "$detail" >>"$RESULTS"
  printf '  %-4s %s%s\n' "$status" "$check" "${detail:+ — $detail}"
}

say() { printf '\n==> %s\n' "$*"; }
die() { record FAIL "$1" "${2:-}"; exit 1; }

summary() {
  local fails
  printf '\n%-14s %-5s %-52s %s\n' PHASE STATUS CHECK DETAIL
  printf '%s\n' "$(printf '%.0s─' {1..120})"
  awk -F'\t' '{ d=$4; if (length(d) > 60) d = substr(d, 1, 57) "..."; printf "%-14s %-5s %-52s %s\n", $1, $3, substr($2, 1, 52), d }' "$RESULTS"
  fails=$(awk -F'\t' '$3=="FAIL"' "$RESULTS" | wc -l | tr -d ' ')
  printf '\n%s checks, %s failed, %s warn, %s gap  (%s)\n' \
    "$(wc -l <"$RESULTS" | tr -d ' ')" "$fails" \
    "$(awk -F'\t' '$3=="WARN"' "$RESULTS" | wc -l | tr -d ' ')" \
    "$(awk -F'\t' '$3=="GAP"' "$RESULTS" | wc -l | tr -d ' ')" "$RESULTS"
  [ "$fails" = 0 ]
}

# ── Host servers ─────────────────────────────────────────────────────────────

free_port() { python3 -c 'import socket; s=socket.socket(); s.bind(("127.0.0.1",0)); print(s.getsockname()[1])'; }

wait_url() { # url seconds
  local i
  for ((i = 0; i < $2; i++)); do curl -fsS -m 2 "$1" >/dev/null 2>&1 && return 0; sleep 1; done
  return 1
}

PIDS=()

start_mirror() { # dir → MIRROR_PORT
  MIRROR_PORT="$(free_port)"
  python3 -m http.server "$MIRROR_PORT" --bind 127.0.0.1 --directory "$1" >"$RUN/mirror-http.log" 2>&1 &
  PIDS+=($!)
  wait_url "http://127.0.0.1:$MIRROR_PORT/" 10 || die "mirror server" "did not start (see $RUN/mirror-http.log)"
}

start_model() { # → MODEL_PORT
  [ -x "$LLMPOSTER" ] || die "fake model" "no llmposter at $LLMPOSTER (cargo install llmposter --version 0.5.0 --locked --root $GATE_HOME)"
  MODEL_PORT="$(free_port)"
  "$LLMPOSTER" --verbose --fixtures "$GATE_SRC/fake-model.yaml" --bind 127.0.0.1 --port "$MODEL_PORT" >"$RUN/llmposter.log" 2>&1 &
  PIDS+=($!)
  wait_url "http://127.0.0.1:$MODEL_PORT/health" 10 || die "fake model" "llmposter did not start (see $RUN/llmposter.log)"
}

# ── Tart VMs ─────────────────────────────────────────────────────────────────
# At most two macOS VMs may run at once (Apple's licence); the gate runs one.

SCRATCH_VMS=()

ssh_opts=(-i "$SSH_KEY" -o StrictHostKeyChecking=no -o UserKnownHostsFile=/dev/null
          -o LogLevel=ERROR -o ConnectTimeout=5 -o ServerAliveInterval=15)

ensure_key() {
  mkdir -p "$GATE_HOME"
  [ -f "$SSH_KEY" ] || ssh-keygen -q -t ed25519 -N '' -C linggen-gate -f "$SSH_KEY"
}

vm_state() { tart list --format json 2>/dev/null | jq -r --arg n "$1" '.[] | select(.Name == $n) | .State'; }

vm_ip() { tart ip "$1" --wait 5 2>/dev/null; }

vm_ssh() { # vm cmd…  (stdin passes through)
  local ip; ip="$(vm_ip "$1")"; shift
  ssh "${ssh_opts[@]}" "admin@$ip" "$@"
}

vm_scp() { # vm src… dest(in VM)
  local vm="$1" ip; ip="$(vm_ip "$vm")"; shift
  local dest="${!#}" n=$(( $# - 1 ))
  scp -q -r "${ssh_opts[@]}" "${@:1:$n}" "admin@$ip:$dest"
}

# Boot headless, wait for SSH, install the gate key (admin/admin once).
vm_boot() { # vm
  local vm="$1" ip i askpass
  tart run --no-graphics "$vm" >"$RUN/tart-$vm.log" 2>&1 &
  PIDS+=($!)
  ip="$(tart ip "$vm" --wait 180 2>/dev/null)" || die "boot $vm" "no IP after 180 s"
  askpass="$GATE_HOME/askpass.sh"
  printf '#!/bin/sh\necho admin\n' >"$askpass"; chmod +x "$askpass"
  for ((i = 0; i < 60; i++)); do
    if ssh "${ssh_opts[@]}" -o BatchMode=yes "admin@$ip" true 2>/dev/null; then return 0; fi
    if SSH_ASKPASS="$askpass" SSH_ASKPASS_REQUIRE=force DISPLAY=:0 \
       ssh -o StrictHostKeyChecking=no -o UserKnownHostsFile=/dev/null -o LogLevel=ERROR \
           -o ConnectTimeout=5 -o PubkeyAuthentication=no "admin@$ip" \
           'mkdir -p ~/.ssh && chmod 700 ~/.ssh && cat >> ~/.ssh/authorized_keys && chmod 600 ~/.ssh/authorized_keys' \
           <"$SSH_KEY.pub" 2>/dev/null; then
      ssh "${ssh_opts[@]}" -o BatchMode=yes "admin@$ip" true 2>/dev/null && return 0
    fi
    sleep 3
  done
  die "boot $vm" "SSH never answered at $ip"
}

# Reverse tunnels: the VM's 127.0.0.1:18765/18766 reach the host's mirror and
# fake model; a local forward exposes the VM's 9527 to Playwright → UI_PORT.
vm_tunnels() { # vm
  local ip; ip="$(vm_ip "$1")"
  UI_PORT="$(free_port)"
  ssh "${ssh_opts[@]}" -N -o ExitOnForwardFailure=yes \
    -R "127.0.0.1:$VM_MIRROR_PORT:127.0.0.1:$MIRROR_PORT" \
    -R "127.0.0.1:$VM_MODEL_PORT:127.0.0.1:$MODEL_PORT" \
    -L "127.0.0.1:$UI_PORT:127.0.0.1:9527" "admin@$ip" >"$RUN/tunnel-$1.log" 2>&1 &
  TUNNEL_PID=$!
  PIDS+=($TUNNEL_PID)
  sleep 2
  vm_ssh "$1" "curl -fsS -m 5 $VM_MIRROR/ >/dev/null" || die "tunnels $1" "the VM cannot reach the mirror (see $RUN/tunnel-$1.log)"
}

vm_untunnel() { [ -n "${TUNNEL_PID:-}" ] && kill "$TUNNEL_PID" 2>/dev/null; TUNNEL_PID=""; }

vm_shutdown() { # vm — clean OS shutdown so the disk is consistent, then stop
  vm_untunnel
  vm_ssh "$1" 'sudo -n shutdown -h now' >/dev/null 2>&1 || true
  local i
  for ((i = 0; i < 60; i++)); do
    [ "$(vm_state "$1")" = running ] || return 0
    sleep 2
  done
  tart stop "$1" >/dev/null 2>&1 || true
}

vm_clone() { # src dst — scratch VM, deleted on exit
  tart delete "$2" >/dev/null 2>&1 || true
  tart clone "$1" "$2" || die "clone $2" "tart clone $1 failed"
  SCRATCH_VMS+=("$2")
}

# Run a VM-side script (scripts/gate/vm/<name>.sh, staged with the rest of
# what the VM gets in $VMFILES) with env; its RESULT lines
# become records, the rest goes to the run log. common.sh's EXIT trap prints
# GATE_EXIT<TAB>rc last: no such line (ssh dropped, VM died) or rc != 0
# (set -u abort, a crash) means checks after that point never ran.
vm_run() { # vm script [VAR=value…]
  local vm="$1" script="$2" line status check detail ssh_rc done_file; shift 2
  done_file="$RUN/vm-$vm.$script.exit"
  rm -f "$done_file"
  vm_ssh "$vm" "rm -rf ~/gate && mkdir -p ~/gate" || die "copy scripts" "ssh failed"
  vm_scp "$vm" "$VMFILES/." "~/gate/" || die "copy scripts" "scp failed"
  vm_ssh "$vm" "cd ~/gate && env $* bash ./$script" 2>&1 | tee -a "$RUN/vm-$vm.log" | \
  while IFS= read -r line; do
    case "$line" in
      RESULT$'\t'*)
        IFS=$'\t' read -r _ status check detail <<<"$line"
        record "$status" "$check" "$detail" ;;
      GATE_EXIT$'\t'*) printf '%s\n' "${line##*$'\t'}" >"$done_file" ;;
    esac
  done
  ssh_rc="${PIPESTATUS[0]}"
  local rc; rc="$(cat "$done_file" 2>/dev/null)"
  if [ -z "$rc" ]; then
    record FAIL "$script ran to the end" "no exit line — ssh exited $ssh_rc mid-run (see $RUN/vm-$vm.log)"
  elif [ "$rc" != 0 ] || [ "$ssh_rc" != 0 ]; then
    record FAIL "$script ran to the end" "exited $rc (ssh $ssh_rc) — checks after the last row never ran (see $RUN/vm-$vm.log)"
  else
    record PASS "$script ran to the end" "exit 0"
  fi
  # Bring the VM's logs home for a look after the VM is gone.
  mkdir -p "$RUN/vm-$vm"
  vm_ssh "$vm" "bash -c 'cd ~ && tar -czf - gate/*.log gate/*.json gate/*.txt .linggen/ling.log .linggen/logs .linggen/config 2>/dev/null'" \
    | tar -xzf - -C "$RUN/vm-$vm" 2>/dev/null || true
}

cleanup() {
  local rc=$? vm p
  vm_untunnel
  for vm in "${SCRATCH_VMS[@]+"${SCRATCH_VMS[@]}"}"; do
    tart stop "$vm" >/dev/null 2>&1 || true
    tart delete "$vm" >/dev/null 2>&1 || true
  done
  for p in "${PIDS[@]+"${PIDS[@]}"}"; do kill "$p" 2>/dev/null || true; done
  exit "$rc"
}
