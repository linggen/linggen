// Answering a waiting question as the person: the one action every card's
// buttons take — the chat's, a subagent pane's, and Yinyue's bubble when she
// tells of the wait. A second tap while the first is in flight does nothing;
// a failure is shown to the person and the card stays.
import { useInteractionStore } from '../stores/interactionStore';
import { useUiStore } from '../stores/uiStore';
import { getTransport } from './transport';
import type { AskUserAnswer } from '../types';

const answering = new Set<string>();

/** Answer `questionId` in `sessionId`. True when the question is settled —
 *  answered now, or gone already (answered elsewhere, expired); false when
 *  the answer did not go through (the card stays) or one is in flight. */
export async function answerAsk(sessionId: string | null, questionId: string, answers: AskUserAnswer[]): Promise<boolean> {
  const key = JSON.stringify([sessionId, questionId]);
  if (answering.has(key)) return false;
  answering.add(key);
  const settle = () => { if (sessionId) useInteractionStore.getState().removeAsk(sessionId, questionId); };
  try {
    await getTransport().sendAskUserResponse({ question_id: questionId, answers, session_id: sessionId });
    settle();
    return true;
  } catch (e) {
    const reason = String((e as Error)?.message ?? e);
    if (/unknown question|already expired/i.test(reason)) {
      settle();
      useUiStore.getState().addToast({ message: 'This question has expired or was already answered.', variant: 'error' });
      return true;
    }
    useUiStore.getState().addToast({ message: `Answer failed to send — ${reason}`, variant: 'error' });
    console.error('Error responding to AskUser:', e);
    return false;
  } finally {
    answering.delete(key);
  }
}
