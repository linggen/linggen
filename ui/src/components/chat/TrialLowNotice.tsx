import React from 'react';
import { account } from '../../lib/endpoints';
import { language } from '../../lib/i18n';

/** Shown once per browser: past this share of the free trial spent, the chat says so. */
const LOW_AT = 0.8;
const SHOWN_KEY = 'linggen.trialLowShown';

/** Fixed words, zh and en — a state notice beside the top bar's meter, not a
 *  turn: the page shows the account's fact itself, no model words it. */
const WORDS = {
  en: 'About a fifth of your free tokens left.',
  zh: '免费额度还剩约两成。',
};

function shownBefore(): boolean {
  try { return localStorage.getItem(SHOWN_KEY) === '1'; } catch { return false; }
}
function markShown(): void {
  try { localStorage.setItem(SHOWN_KEY, '1'); } catch { /* private window: it may show again */ }
}

/** Whether the account is on its trial with ≥80% of it spent and some left. */
async function trialLow(): Promise<boolean> {
  const acc = await account.get('linggen').catch(() => null);
  const gate = acc?.gate;
  if (!acc?.signed_in || !gate || gate.developer || gate.entitled) return false;
  const t = gate.trial;
  if (!t || !t.budget || !t.active) return false;
  return t.tokens / t.budget >= LOW_AT;
}

/** The one-time "free tokens running low" line above the chat input
 *  (Hanli 2026-11-03: trial 5M tokens; agent's pick: once per browser). */
export const TrialLowNotice: React.FC<{ busy: boolean }> = ({ busy }) => {
  const [show, setShow] = React.useState(false);
  React.useEffect(() => {
    if (busy || show || shownBefore()) return;
    let live = true;
    trialLow().then((low) => {
      if (!live || !low) return;
      markShown();
      setShow(true);
    });
    return () => { live = false; };
  }, [busy, show]);
  if (!show) return null;
  return (
    <div data-testid="trial-low" className="mx-3 mb-1 flex items-center gap-2 rounded-md bg-amber-500/10 px-3 py-1.5 text-xs text-amber-700 dark:text-amber-400">
      <span className="flex-1">{language() === 'zh-Hans' ? WORDS.zh : WORDS.en}</span>
      <button type="button" onClick={() => setShow(false)} className="opacity-70 hover:opacity-100" aria-label="Close">&#x2715;</button>
    </div>
  );
};
