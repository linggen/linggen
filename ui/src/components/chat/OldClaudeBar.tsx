/**
 * In place of the composer for an old Claude session (lib/oldClaude.mts):
 * one line saying why it is read-only, and the one way on.
 */
import React, { useState } from 'react';
import { OLD_CLAUDE_LINE, continuedLine } from '../../lib/oldClaude.mts';
import { apiErrorMessage } from '../../lib/api';
import { useSessionStore } from '../../stores/sessionStore';
import { useUiStore } from '../../stores/uiStore';

export const OldClaudeBar: React.FC<{ sessionId: string }> = ({ sessionId }) => {
  const [busy, setBusy] = useState(false);
  const go = async () => {
    setBusy(true);
    try {
      const model = await useSessionStore.getState().continueOldClaude(sessionId);
      if (model) useUiStore.getState().addToast({ variant: 'info', message: continuedLine(model) });
    } catch (error) {
      useUiStore.getState().addToast({ variant: 'error', message: apiErrorMessage(error) });
    } finally {
      setBusy(false);
    }
  };
  return (
    <div data-testid="old-claude-bar" className="mx-3 mb-3 flex flex-wrap items-center gap-3 rounded-lg border border-slate-200 dark:border-white/10 px-3 py-2 text-[13px] text-slate-600 dark:text-slate-300">
      <span className="flex-1 min-w-0">{OLD_CLAUDE_LINE}</span>
      <button
        onClick={() => { void go(); }}
        disabled={busy}
        className="shrink-0 rounded-md bg-slate-900 dark:bg-white px-3 py-1 text-[12px] font-medium text-white dark:text-slate-900 disabled:opacity-50"
      >
        Continue in Linggen
      </button>
    </div>
  );
};
