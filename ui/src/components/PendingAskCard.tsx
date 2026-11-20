/**
 * A waiting question's card — the one implementation of its answer buttons,
 * wherever it shows: the chat, a subagent's pane, and Yinyue's bubble when
 * she tells of the wait (`YinyueBubble`). A permission prompt gets the
 * permission card, any other question the question card; a wait she told of
 * aloud carries her folded mark.
 */
import React from 'react';
import type { AskUserAnswer, PendingAskUser } from '../types';
import { AskUserCard } from './AskUserCard';
import { ToolPermissionCard } from './ToolPermissionCard';
import { ToldAloudMark } from './chat/ToldAloudMark';
import { isPermissionAsk } from '../lib/askQueue.mts';

export const PendingAskCard: React.FC<{
  pending: PendingAskUser;
  onRespond: (questionId: string, answers: AskUserAnswer[]) => void;
}> = ({ pending, onRespond }) => (
  <>
    {isPermissionAsk(pending)
      ? <ToolPermissionCard key={pending.questionId} pending={pending} onRespond={onRespond} />
      : <AskUserCard key={pending.questionId} pending={pending} onRespond={onRespond} />}
    <ToldAloudMark told={pending.toldAloud} />
  </>
);
