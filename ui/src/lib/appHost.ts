/**
 * The one place that knows whether this page is inside the desktop shell, and
 * the one place that opens an app.
 *
 * The desktop app's window shows this same UI. Everything that differs from a
 * browser asks `inShell()` — the only reader of the shell's IPC object — and
 * never a query or a face name (doc/code-style.md, principle 2; the lock is
 * ui/tests/oneShell.test.mts).
 *
 * Opening an app (a skill card's click, an `app_launched` event, an offer
 * Yinyue made) goes through `openApp`: inside the shell the app's tab comes
 * to the front (apps/AppTabs.tsx registers how) and the window is raised; in a
 * browser the app opens in a new browser tab, through the relay when this page
 * was reached through it. The window is the shell's: shown, unminimized and
 * focused through the shell's IPC, which the shell grants this page for
 * exactly those three (linggen-app shell/capabilities/main-remote.json).
 */
import type { SkillInfo } from '../types';
import { useSessionStore } from '../stores/sessionStore';

type TauriIpc = { invoke: (cmd: string, args?: Record<string, unknown>) => Promise<unknown> };

const shellIpc = () => (window as { __TAURI_INTERNALS__?: TauriIpc }).__TAURI_INTERNALS__;

/** True inside the desktop app's window: the one question every difference asks. */
export const inShell = () => !!shellIpc();

/** What the tab bar can do: bring an app's tab forward (at one of its own
 *  views), or the chat home. Only apps/AppTabs.tsx registers one. */
export type AppTabHost = {
  open: (skill: string, view?: string) => boolean;
  home: () => void;
};

let host: AppTabHost | null = null;

export function setAppHost(next: AppTabHost | null): void {
  host = next;
}

async function raiseWindow(): Promise<void> {
  const ipc = shellIpc();
  if (!ipc?.invoke) return;
  for (const cmd of ['show', 'unminimize', 'set_focus']) {
    await ipc.invoke(`plugin:window|${cmd}`, { label: 'main' }).catch(() => {});
  }
}

/** The view a launch URL names (`?view=`), for the tab to land on. */
export function viewOf(url: string): string | undefined {
  const query = url.split('?')[1] ?? '';
  return new URLSearchParams(query).get('view') || undefined;
}

/** The page a skill's app opens at, or null when it is no page (a `bash` app runs as a chat command). */
export function appPage(skill: Pick<SkillInfo, 'name' | 'app'>): string | null {
  const app = skill.app;
  if (app?.launcher === 'web') return `/apps/${skill.name}/${app.entry}`;
  if (app?.launcher === 'url') return app.entry;
  return null;
}

export type AppLaunch = {
  skill: string;
  /** The page: `/apps/<skill>/…` on this engine, or an outside address. */
  url: string;
  /** Set for an `app_launched` event: the session that launched it. A browser
   *  opens only a launch from the session it shows; the event is broadcast. */
  launchedBy?: string | null;
};

const surface = () => (window as { __LINGGEN_VIEW__?: string }).__LINGGEN_VIEW__ || 'main';

/** A page of this engine, reached through the relay when this page was. */
function browserUrl(url: string): string {
  const instance = document.querySelector('meta[name="linggen-instance"]')?.getAttribute('content');
  if (!instance || !url.startsWith('/')) return url;
  const relay = document.querySelector('meta[name="linggen-relay-origin"]')?.getAttribute('content') || '';
  return `${relay}/app/connect/${instance}?app=${encodeURIComponent(url)}`;
}

/** Open an app where this window opens apps. False when this surface does not
 *  open it (a pet window, an embed, another session's launch, no such tab). */
export function openApp(launch: AppLaunch): boolean {
  // The pet window, embeds and channels leave it to the window that shows
  // apps; a room guest's page opens only what its own skill cards ask for.
  const where = surface();
  if (where !== 'main' && !(where === 'consumer' && launch.launchedBy === undefined)) return false;
  const own = launch.url.startsWith('/');
  if (inShell() && own) {
    if (!host || !host.open(launch.skill, viewOf(launch.url))) return false;
    void raiseWindow();
    return true;
  }
  if (!inShell() && launch.launchedBy !== undefined && launch.launchedBy !== useSessionStore.getState().activeSessionId) {
    return false;
  }
  window.open(browserUrl(launch.url), '_blank');
  return true;
}

/** Bring the chat home forward — after something chose a session to show. */
export function showChatHome(): void {
  host?.home();
}
