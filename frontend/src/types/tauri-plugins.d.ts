// Minimal ambient module declarations for Tauri packages.
//
// These keep TypeScript happy in environments where the editor/linter cannot
// resolve `node_modules` (e.g. sandboxed tooling). Runtime still relies on the
// actual installed packages.

declare module '@tauri-apps/plugin-updater' {
  export type DownloadEvent = { event: 'Started' | 'Progress' | 'Finished' }

  export type Update = {
    version?: string
    date?: string
    body?: string
    downloadAndInstall: (cb?: (event: DownloadEvent) => void) => Promise<void>
  }

  export function check(): Promise<Update | null>
}

declare module '@tauri-apps/plugin-process' {
  export function relaunch(): Promise<void>
}

declare module '@tauri-apps/api/app' {
  export function getVersion(): Promise<string>
}
