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

/** Opens `skill`'s tab — at `view`, one of that page's own views, when the launch names one. */
type OpenApp = (skill: string, view?: string) => boolean;

let host: OpenApp | null = null;

export function setAppHost(open: OpenApp | null): void {
  host = open;
}

type TauriIpc = { invoke: (cmd: string, args?: Record<string, unknown>) => Promise<unknown> };

const shellIpc = () => (window as { __TAURI_INTERNALS__?: TauriIpc }).__TAURI_INTERNALS__;
export const inShell = () => !!shellIpc();

async function raiseWindow(): Promise<void> {
  const ipc = shellIpc();
  if (!ipc?.invoke) return;
  for (const cmd of ['show', 'unminimize', 'set_focus']) {
    await ipc.invoke(`plugin:window|${cmd}`, { label: 'main' }).catch(() => {});
  }
}

/** The view a launch URL names (`?view=`), for the host to land on. */
export function viewOf(url: string): string | undefined {
  const query = url.split('?')[1] ?? '';
  return new URLSearchParams(query).get('view') || undefined;
}

/** Open the app's tab and bring the window up. False: no host, or no such app. */
export function launchInHost(skill: string, view?: string): boolean {
  if (!host || !host(skill, view)) return false;
  void raiseWindow();
  return true;
}
