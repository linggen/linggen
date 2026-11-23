/**
 * A line said to Yinyue through her avatar door (`POST /api/yinyue/chat`): the
 * one place it is sent, for the line typed into her box and the line heard
 * from a held press (`asked_aloud`, shared/hold-to-talk.js). She ponders until her
 * reply arrives over the event spine (handlePetSpeak clears it), or 30 s pass.
 */
import { useUiStore } from '../stores/uiStore';
import type { LineSent } from '@linggen/hold-to-talk';

const PONDER_LIMIT_MS = 30_000;
let ponderTimer = 0;

/** Whether the line went; when it did not (the pondering is undone), her door's
 *  own reason if it gave one — a spoken line is refused where a typed one is not. */
export async function askYinyue(text: string, askedAloud = false): Promise<LineSent> {
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
    if (!resp.ok) return refused(resp);
    return { sent: true };
  } catch (e) {
    console.error('[yinyue] chat failed', e);
    setPetThinking(false);
    window.clearTimeout(ponderTimer);
    return { sent: false, reason: null };
  }
}

/** Her door said no: undo the pondering, and carry its reason when it gave one. */
async function refused(resp: Response): Promise<LineSent> {
  const body = (await resp.json().catch(() => null)) as { reason?: unknown } | null;
  useUiStore.getState().setPetThinking(false);
  window.clearTimeout(ponderTimer);
  console.error(`[yinyue] chat refused ${resp.status}`);
  return { sent: false, reason: typeof body?.reason === 'string' && body.reason ? body.reason : null };
}
