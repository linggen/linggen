/**
 * The surface that hosts apps — the desktop launcher — and what an
 * `app_launched` asks of it: that app's tab in front, and its window up.
 *
 * The launcher registers how it opens a tab; the event handler calls it. The
 * window is the desktop shell's: shown, unminimized and focused through the
 * shell's IPC, which the shell grants this page for exactly those three
 * (linggen-app shell/capabilities/main-remote.json). Outside the shell there
 * is no IPC and nothing to raise.
 */

type OpenApp = (skill: string) => boolean;

let host: OpenApp | null = null;

export function setAppHost(open: OpenApp | null): void {
  host = open;
}

type TauriIpc = { invoke: (cmd: string, args?: Record<string, unknown>) => Promise<unknown> };

async function raiseWindow(): Promise<void> {
  const ipc = (window as { __TAURI_INTERNALS__?: TauriIpc }).__TAURI_INTERNALS__;
  if (!ipc?.invoke) return;
  for (const cmd of ['show', 'unminimize', 'set_focus']) {
    await ipc.invoke(`plugin:window|${cmd}`, { label: 'main' }).catch(() => {});
  }
}

/** Open the app's tab and bring the window up. False: no host, or no such app. */
export function launchInHost(skill: string): boolean {
  if (!host || !host(skill)) return false;
  void raiseWindow();
  return true;
}
