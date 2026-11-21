/**
 * A line said to Yinyue through her avatar door (`POST /api/yinyue/chat`): the
 * one place it is sent, for the line typed into her box and the line heard
 * from a held press (`asked_aloud`, lib/holdToTalk.mts). She ponders until her
 * reply arrives over the event spine (handlePetSpeak clears it), or 30 s pass.
 */
import { useUiStore } from '../stores/uiStore';

const PONDER_LIMIT_MS = 30_000;
let ponderTimer = 0;

/** True when the line went; false when it did not (the pondering is undone). */
export async function askYinyue(text: string, askedAloud = false): Promise<boolean> {
  const { setPetThinking } = useUiStore.getState();
  setPetThinking(true);
  window.clearTimeout(ponderTimer);
  ponderTimer = window.setTimeout(() => setPetThinking(false), PONDER_LIMIT_MS); // safety net
  try {
    const resp = await fetch('/api/yinyue/chat', {
      method: 'POST',
      headers: { 'Content-Type': 'application/json' },
      body: JSON.stringify(askedAloud ? { text, asked_aloud: true } : { text }),
    });
    if (!resp.ok) throw new Error(`yinyue chat ${resp.status}`);
    return true;
  } catch (e) {
    console.error('[yinyue] chat failed', e);
    setPetThinking(false);
    window.clearTimeout(ponderTimer);
    return false;
  }
}
