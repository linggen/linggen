/**
 * Presence beat — a tiny throttled liveness signal so Yinyue can tell whether
 * the user is here (typing), present but reading, or away.
 *
 * Sends ONLY recency + focus + a typing flag to `POST /api/presence` — never
 * keystroke content — over the data channel, like every call the UI makes.
 * A beat is a reading, not an action: while the channel is down it is skipped
 * rather than queued (the next one says the same, fresher), and the engine
 * ages out a surface that stops beating, so a closed tab needs no last word. Input listeners just stamp timestamps (cheap); the network
 * beat is throttled to a steady cadence plus an immediate beat on focus/visibility
 * change. The server derives the three-state read in the `sense` tool.
 */
import { connection } from './transport';

const BEAT_MS = 4000; // steady cadence
const TYPING_WINDOW_MS = 1500; // counts as "typing" if a key landed this recently

let lastInputAt = Date.now();
let lastKeyAt = 0;
// This page's own id: the engine keeps one reading per surface, so a tab
// beside this one (focused but idle, or blurred) never overwrites it.
const SURFACE = `ui-${Math.random().toString(36).slice(2, 10)}${Date.now().toString(36)}`;

function snapshot(): { surface: string; focused: boolean; typing: boolean; idle_ms: number } {
  const now = Date.now();
  // "Active in Linggen" means Linggen is the focused surface — keystrokes in
  // another app never reach this page, and a blurred/hidden tab reads as away.
  // Gate `typing` on focus so the raw field can't claim activity while the user
  // is off in another window.
  const focused = document.hasFocus() && document.visibilityState === 'visible';
  return {
    surface: SURFACE,
    focused,
    typing: focused && now - lastKeyAt < TYPING_WINDOW_MS,
    idle_ms: now - lastInputAt,
  };
}

function beat(): void {
  if (!connection.isOpen()) return;
  fetch('/api/presence', {
    method: 'POST',
    headers: { 'Content-Type': 'application/json' },
    body: JSON.stringify(snapshot()),
  }).catch(() => {
    /* no surface connected / offline — nothing to do */
  });
}

/** Start the presence beat. Returns a cleanup function. */
export function startPresenceBeat(): () => void {
  const onKey = () => {
    lastInputAt = Date.now();
    lastKeyAt = Date.now();
  };
  const onMove = () => {
    lastInputAt = Date.now();
  };
  const onFocusChange = () => beat(); // surface a change in presence immediately

  window.addEventListener('keydown', onKey, { passive: true });
  window.addEventListener('pointermove', onMove, { passive: true });
  window.addEventListener('focus', onFocusChange);
  window.addEventListener('blur', onFocusChange);
  document.addEventListener('visibilitychange', onFocusChange);

  beat(); // initial
  const timer = window.setInterval(beat, BEAT_MS);

  return () => {
    window.clearInterval(timer);
    window.removeEventListener('keydown', onKey);
    window.removeEventListener('pointermove', onMove);
    window.removeEventListener('focus', onFocusChange);
    window.removeEventListener('blur', onFocusChange);
    document.removeEventListener('visibilitychange', onFocusChange);
  };
}
