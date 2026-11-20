/**
 * What the session top bar says about spend. The decision is lib/spend.mts's;
 * this only draws it: the Claude SDK's figure, an estimate for a priced
 * model, tokens for an unpriced one, the ChatGPT allowance for an account.
 */
import React from 'react';
import { useChatStore } from '../../stores/chatStore';
import { useServerStore } from '../../stores/serverStore';
import { useSessionStore } from '../../stores/sessionStore';
import { useUiStore } from '../../stores/uiStore';
import { spendView } from '../../lib/spend.mts';
import { CodexUsage } from '../CodexUsage';

const TONE = 'text-[11px] text-slate-600 dark:text-slate-300';

export const SessionSpend: React.FC<{ leading: boolean }> = ({ leading }) => {
  const sessionId = useSessionStore((s) => s.activeSessionId) || '';
  const claude = useSessionStore((s) => s.allSessions.find((item) => item.id === sessionId)?.runtime === 'claude_agent_sdk');
  const state = useChatStore((s) => (s._sessionStateLoad?.sessionId === sessionId ? s.sessionState : undefined));
  // The ChatGPT allowance is the account's: shown while this chat runs on a ChatGPT model.
  const chosen = useUiStore((s) => s.sessionModel);
  const fallback = useServerStore((s) => s.defaultModels[0]);
  const modelId = chosen || fallback;
  const onChatGpt = useServerStore((s) => s.models.find((m) => m.id === modelId)?.provider === 'chatgpt');

  const view = spendView({
    claude,
    claudeCost: state?.claude_usage?.estimated_cost_usd,
    onChatGpt,
    usage: state?.usage,
  });
  const lead = leading ? ' · ' : '';
  return (
    <>
      {view.claude && (
        <span className={TONE} data-no-i18n title="Estimated USD cost recorded for this Claude conversation. Includes subagents; final charges follow Anthropic billing.">
          {lead}≈ ${view.claude.cost === null ? '—' : view.claude.cost.toFixed(2)}
        </span>
      )}
      {view.estimate && (
        <span className={TONE} data-testid="session-spend" title={view.estimate.title}>
          <span data-no-i18n>{lead}{view.estimate.text}</span>
        </span>
      )}
      {view.tokens && (
        <span className={TONE} data-testid="session-spend" title={view.tokens.title}>
          {lead}{view.tokens.text}
        </span>
      )}
      {view.allowance && <CodexUsage />}
    </>
  );
};
