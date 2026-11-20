const WINDOW_MARK = 'linggen:launcher-face';

/** Decide the face and the mark to keep for this top-level window. */
export function resolveLauncherFace(search: string, topLevel: boolean, remembered: boolean) {
  const query = new URLSearchParams(search);
  if (!topLevel || query.get('pet') === '1' || query.get('channel') === '1') {
    return { launcher: false, remember: remembered };
  }
  const requested = query.get('launcher');
  const launcher = requested === '1' || (requested !== '0' && remembered);
  return { launcher, remember: launcher };
}

export function isLauncherWindow(win: Window): boolean {
  const search = win.location.search;
  // Same-origin frames share sessionStorage with the top page. Never even
  // read the mark from a frame, including the hidden skill channel.
  if (win !== win.top) return false;
  try {
    const storage = win.sessionStorage;
    const remembered = storage.getItem(WINDOW_MARK) === '1';
    const face = resolveLauncherFace(search, true, remembered);
    if (face.remember !== remembered) {
      if (face.remember) storage.setItem(WINDOW_MARK, '1');
      else storage.removeItem(WINDOW_MARK);
    }
    return face.launcher;
  } catch {
    // Restricted storage must not break the native shell's entry URL.
    return resolveLauncherFace(search, true, false).launcher;
  }
}
