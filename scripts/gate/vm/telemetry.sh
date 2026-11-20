# Shared by the host gate (lib.sh) and guest CLI launches (common.sh).
# The gate also plants a file before running anything in the guest: apps
# launched by macOS do not inherit the SSH shell's environment.
gate_disable_telemetry() {
  export LINGGEN_NO_TELEMETRY=1 LING_MEM_NO_TELEMETRY=1
}

# Guest only. Keep this in the VM's home so it survives update and reboot.
gate_prepare_vm_telemetry() {
  mkdir -p "$HOME/.linggen" && touch "$HOME/.linggen/no-telemetry"
}

gate_vm_telemetry_disabled() {
  [ -f "$HOME/.linggen/no-telemetry" ]
}
