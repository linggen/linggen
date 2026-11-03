import React from 'react';
import { billingReason } from '../../lib/billing.mts';
import { language } from '../../lib/i18n';
import { openModelsSettings, resendOnOwnModel } from '../../lib/ownModel';
import type { ChatMessage } from '../../types';

/** Fixed words, zh and en. The one exception to "Yinyue words every message":
 *  the allowance is spent, so no model can speak this line. */
const WORDS = {
  en: {
    trial: 'Your free tokens are used up.',
    monthly: 'This month’s tokens are used up.',
    how: 'Subscribe, or use your own model.',
    subscribe: 'Subscribe',
    own: 'Use your own model',
    waiting: 'Add a model in Settings — this message sends again on it.',
  },
  zh: {
    trial: '免费额度用完了。',
    monthly: '本月额度用完了。',
    how: '订阅，或者用自己的模型。',
    subscribe: '订阅',
    own: '用自己的模型',
    waiting: '在设置里添加模型后，这条消息会自动重发。',
  },
};

/** Inline card when a turn fails because the account's allowance is spent —
 *  the Linggen Cloud proxy's 402 (free trial used up) or 429 (monthly pool).
 *  Two ways on: subscribe on linggen.dev, or add one's own model; once a model
 *  is added, the refused message is sent again on it. Mirrors
 *  AuthRequiredBlock's plain-sentinel wiring (`BILLING_REQUIRED:`). */
export const BillingRequiredBlock: React.FC<{
  body: string;
  msg?: ChatMessage;
  onResend?: (failed: ChatMessage) => void;
}> = ({ body, msg, onResend }) => {
  const w = language() === 'zh-Hans' ? WORDS.zh : WORDS.en;
  const [waiting, setWaiting] = React.useState(false);
  const stop = React.useRef<AbortController | null>(null);
  React.useEffect(() => () => stop.current?.abort(), []);

  const useOwn = async () => {
    openModelsSettings();
    if (!msg || !onResend || waiting) return;
    setWaiting(true);
    stop.current = new AbortController();
    const sent = await resendOnOwnModel(msg, onResend, stop.current.signal);
    if (!sent) setWaiting(false);
  };

  return (
    <div data-testid="billing-required" className="rounded-lg border border-blue-300 dark:border-blue-800 bg-blue-50 dark:bg-blue-950/40 px-4 py-3 text-sm text-blue-900 dark:text-blue-200">
      <div className="flex items-start gap-2">
        <span className="mt-0.5 shrink-0 text-blue-500 dark:text-blue-400">&#x2728;</span>
        <div className="space-y-2">
          <div>{w[billingReason(body)]} {w.how}</div>
          <div className="flex flex-wrap gap-2">
            <a
              href="https://linggen.dev/app/billing"
              target="_blank"
              rel="noopener noreferrer"
              className="inline-block rounded-md bg-blue-600 hover:bg-blue-700 px-3 py-1.5 text-white text-xs font-medium"
            >
              {w.subscribe}
            </a>
            <button
              type="button"
              onClick={useOwn}
              className="inline-block rounded-md border border-blue-400 dark:border-blue-700 hover:bg-blue-100 dark:hover:bg-blue-900/50 px-3 py-1.5 text-xs font-medium"
            >
              {w.own}
            </button>
          </div>
          {waiting && <div className="text-xs opacity-80">{w.waiting}</div>}
        </div>
      </div>
    </div>
  );
};
