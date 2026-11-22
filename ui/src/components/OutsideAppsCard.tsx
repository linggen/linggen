import React from 'react';
import { apiErrorMessage } from '../lib/api';
import { outsideAppsApi } from '../lib/endpoints';
import { WORDS, openAfterToggle, type OutsideAppsReply } from '../lib/outsideApps.mts';
import { Section } from './settings/kit';

/**
 * The private apps the owner may open to agents outside Linggen, one switch
 * each. The engine's list is the list: nothing here names an app, and with
 * none that may be opened the section is not shown.
 */
export const OutsideAppsCard: React.FC = () => {
  const [reply, setReply] = React.useState<OutsideAppsReply | null>(null);
  const [busy, setBusy] = React.useState(false);
  const [error, setError] = React.useState<string | null>(null);

  const refresh = React.useCallback(async () => {
    try {
      setReply(await outsideAppsApi.list());
    } catch {
      setReply(null);
    }
  }, []);
  React.useEffect(() => {
    refresh();
  }, [refresh]);

  if (!reply || reply.apps.length === 0) return null;
  const toggle = async (name: string, on: boolean) => {
    setBusy(true);
    setError(null);
    try {
      await outsideAppsApi.set(openAfterToggle(reply.apps, name, on));
    } catch (e) {
      setError(apiErrorMessage(e));
    } finally {
      await refresh();
      setBusy(false);
    }
  };
  return (
    <Section title={WORDS.title} desc={WORDS.hint}>
      {error && <p role="alert" className="text-xs text-red-500 mb-2">{error}</p>}
      <ul className="space-y-1">
        {reply.apps.map((app) => (
          <li
            key={app.name}
            data-testid="outside-app"
            data-app={app.name}
            className="flex items-center gap-3 px-3 py-2.5 bg-slate-50 dark:bg-white/[0.02] rounded-lg border border-slate-100 dark:border-white/5"
          >
            <span className="text-xs font-mono font-semibold text-slate-700 dark:text-slate-200 w-40 shrink-0">{app.name}</span>
            <span className="text-[12px] text-slate-500 dark:text-slate-400 flex-1 min-w-0 truncate">{app.description}</span>
            <button
              role="switch"
              aria-checked={app.open}
              aria-label={`${WORDS.switch}: ${app.name}`}
              disabled={busy || !reply.at_this_mac}
              onClick={() => toggle(app.name, !app.open)}
              className={`shrink-0 px-2 py-1 rounded-md text-[11px] font-medium border transition-colors disabled:opacity-60 ${
                app.open
                  ? 'bg-blue-600/10 border-blue-500/40 text-blue-600 dark:text-blue-400'
                  : 'border-slate-200 dark:border-white/10 text-slate-500'
              }`}
            >
              {app.open ? '✓ ' : ''}{WORDS.switch}
            </button>
          </li>
        ))}
      </ul>
    </Section>
  );
};
