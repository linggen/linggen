import React from 'react';
import { MarkdownContent } from './MarkdownContent';

/** Inline subscribe CTA shown when a turn fails because the account's
 *  allowance is spent — the Linggen Cloud proxy's 402 (free trial used up,
 *  or monthly pool exhausted). Mirrors AuthRequiredBlock's plain-sentinel
 *  wiring: the engine prefixes the proxy's message with `BILLING_REQUIRED:`.
 */
export const BillingRequiredBlock: React.FC<{ message: string }> = ({ message }) => (
  <div className="rounded-lg border border-blue-300 dark:border-blue-800 bg-blue-50 dark:bg-blue-950/40 px-4 py-3 text-sm text-blue-900 dark:text-blue-200">
    <div className="flex items-start gap-2">
      <span className="mt-0.5 shrink-0 text-blue-500 dark:text-blue-400">&#x2728;</span>
      <div className="space-y-2">
        <MarkdownContent text={message} />
        <a
          href="https://linggen.dev/app/billing"
          target="_blank"
          rel="noopener noreferrer"
          className="inline-block rounded-md bg-blue-600 hover:bg-blue-700 px-3 py-1.5 text-white text-xs font-medium"
        >
          Subscribe
        </a>
      </div>
    </div>
  </div>
);
