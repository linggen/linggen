const WINDOW_MARK = 'linggen:launcher-face';

/** Decide the face and the mark to keep for this top-level window. */
export function isEmbedLocation(path: string, query: URLSearchParams) {
  // The relay's connect page can also be marked by the skill chat bridge.
  return path === '/embed' || path.startsWith('/embed/')
    || query.get('mode') === 'compact' || query.get('entry') === 'embed';
}

export function resolveLauncherFace(search: string, topLevel: boolean, remembered: boolean, path = '/') {
  if (!topLevel) return { launcher: false, remember: remembered };
  const query = new URLSearchParams(search);
  const requested = query.get('launcher');
  if (requested === '1') return { launcher: true, remember: true };
  if (query.get('pet') === '1' || query.get('channel') === '1' || isEmbedLocation(path, query)) {
    return { launcher: false, remember: remembered };
  }
  const launcher = requested !== '0' && remembered;
  return { launcher, remember: launcher };
}

export function isLauncherWindow(win: Window): boolean {
  const { search, pathname } = win.location;
  // Same-origin frames share sessionStorage with the top page. Never even
  // read the mark from a frame, including the hidden skill channel.
  if (win !== win.top) return false;
  try {
    const storage = win.sessionStorage;
    const remembered = storage.getItem(WINDOW_MARK) === '1';
    const face = resolveLauncherFace(search, true, remembered, pathname);
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
