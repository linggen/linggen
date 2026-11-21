/**
 * Hold to talk, bound to this page: the one `HoldToTalk` (lib/holdToTalk.mts)
 * with the page's real microphone, fetch, timers and window, her door
 * (askYinyue) and her one stop (hush). The page avatar and the pet window both
 * draw from it (components/yinyue/YinyueAvatar.tsx).
 */
import { useSyncExternalStore } from 'react';
import { useUiStore } from '../stores/uiStore';
import { askYinyue } from './askYinyue';
import { hush, setPersonTalking } from './eventHandlers/yinyue';
import { createHoldToTalk, talkLabel, type TalkView } from './holdToTalk.mts';
import { newSpeechContext } from './speechTap';

/** A stage frame (`?pet=1&stage=1`, a scene that stands her in a place) is never
 *  granted the microphone — the app's gate and the frame's own policy deny it —
 *  so holding her there is not offered: a press stays a tap. */
const IN_STAGE_FRAME = new URLSearchParams(window.location.search).get('stage') === '1';

export const talk = createHoldToTalk({
  mediaDevices: () => navigator.mediaDevices,
  newAudioContext: newSpeechContext,
  offered: () => !IN_STAGE_FRAME,
  fetch: (url, init) => window.fetch(url, init),
  sendLine: (text) => askYinyue(text, true),
  cutOff: hush,
  now: () => Date.now(),
  setTimer: (fn, ms) => window.setTimeout(fn, ms),
  clearTimer: (id) => window.clearTimeout(id as number),
  win: window,
  doc: {
    get hidden() { return document.hidden; },
    addEventListener: (type, fn) => document.addEventListener(type, fn),
    removeEventListener: (type, fn) => document.removeEventListener(type, fn),
  },
});

// The pet window's native drag (linggen-app shell/src/pet.rs) stands down while
// a hold is on the page, so a hold never turns into a window drag mid-sentence.
const HOLDING = new Set(['opening', 'listening', 'refused']);
talk.subscribe(() => {
  const root = document.documentElement;
  const holding = HOLDING.has(talk.view().phase);
  // While held, a line of hers is shown and not voiced (eventHandlers/yinyue.ts).
  setPersonTalking(holding);
  if (holding) root.dataset.talking = '1';
  else delete root.dataset.talking;
});

/** What the avatar draws for the talk: the state's words, what was heard, one quiet line. */
export function useTalk(): { view: TalkView; label: string | null } {
  const view = useSyncExternalStore(talk.subscribe, talk.view);
  const pondering = useUiStore((s) => s.petThinking);
  return { view, label: talkLabel(view, pondering) };
}
