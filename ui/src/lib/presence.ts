/**
 * Presence beat — a tiny throttled liveness signal so Yinyue can tell whether
 * the user is here (typing), present but reading, or away.
 *
 * Sends ONLY recency + focus + a typing flag (and, for an app's chat, the app's
 * name) to `POST /api/presence` — never keystroke content — over the data
 * channel, like every call the UI makes.
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

// The skill this surface shows, when it is an app's chat (the embed under an
// app page); none on the main page.
let app: string | null = null;
let covered = false;
let reportedFocus = false;

/** The shell's app tab covers this surface. Withdraw the last focused
 * reading now; while covered, only a pending withdrawal may beat. */
export function setPresenceCovered(next: boolean): void {
  if (covered === next) return;
  covered = next;
  beat();
}

function snapshot(): { surface: string; focused: boolean; typing: boolean; idle_ms: number; app?: string } {
  const now = Date.now();
  // "Active in Linggen" means Linggen is the focused surface — keystrokes in
  // another app never reach this page, and a blurred/hidden tab reads as away.
  // Gate `typing` on focus so the raw field can't claim activity while the user
  // is off in another window.
  const focused = !covered && document.hasFocus() && document.visibilityState === 'visible';
  return {
    surface: SURFACE,
    focused,
    typing: focused && now - lastKeyAt < TYPING_WINDOW_MS,
    idle_ms: now - lastInputAt,
    ...(app ? { app } : {}),
  };
}

function beat(): void {
  if (!connection.isOpen() || (covered && !reportedFocus)) return;
  const reading = snapshot();
  reportedFocus = reading.focused;
  fetch('/api/presence', {
    method: 'POST',
    headers: { 'Content-Type': 'application/json' },
    body: JSON.stringify(reading),
  }).catch(() => {
    /* no surface connected / offline — nothing to do */
  });
}

/**
 * Start the presence beat. `skill` names the app this surface belongs to — an
 * app's chat, whose keys never reach the app page around it. Returns a
 * cleanup function.
 */
export function startPresenceBeat(skill?: string): () => void {
  app = skill || null;
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
