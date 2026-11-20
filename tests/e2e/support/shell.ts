// A stand-in for the desktop app (linggen-app's shell): it gives the page the
// shell's IPC object, which is the one fact the UI asks about (`inShell()`,
// ui/src/lib/appHost.ts), and records what the page asks of it. The real
// shell is assumed to give it to the window's top page only — the frames
// inside it (app pages, their chats) are left without it unless `allFrames`.
import type { Page } from '@playwright/test';

export async function standInForShell(page: Page, { allFrames = false } = {}) {
  await page.addInitScript((everyFrame: boolean) => {
    if (!everyFrame && window !== window.top) return;
    const calls: string[] = [];
    Object.defineProperty(window, '__shellCalls', { value: calls });
    Object.defineProperty(window, '__TAURI_INTERNALS__', {
      value: { invoke: async (cmd: string) => { calls.push(cmd); return null; } },
    });
  }, allFrames);
}

/** What the page asked of the shell's IPC, in order. */
export function shellCalls(page: Page): Promise<string[]> {
  return page.evaluate(() => (window as unknown as { __shellCalls?: string[] }).__shellCalls ?? []);
}
