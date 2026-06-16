import React from 'react';
import { MarkdownContent } from './MarkdownContent';

type LoginState = 'idle' | 'waiting' | 'done';

/** Per-provider re-auth wiring. Each reuses the daemon's browser-login + poll
 *  flow (the same one Settings uses), so the user can re-authenticate inline
 *  without leaving chat. `chatgpt` = Codex OAuth; `linggen` = linggen.dev
 *  account (the Linggen Cloud models). */
const LOGIN_FLOWS: Record<string, {
  loginUrl: string;
  statusUrl: string;
  isDone: (data: { authenticated?: boolean; signed_in?: boolean }) => boolean;
  label: string;
}> = {
  chatgpt: {
    loginUrl: '/api/auth/codex/login',
    statusUrl: '/api/auth/codex/status',
    isDone: (d) => !!d.authenticated,
    label: 'Sign in with ChatGPT',
  },
  linggen: {
    loginUrl: '/api/account/login',
    statusUrl: '/api/account',
    isDone: (d) => !!d.signed_in,
    label: 'Sign in',
  },
};

/** Inline sign-in CTA shown when a turn fails because the model needs auth
 *  (OAuth session expired, or no linggen.dev sign-in). Lets the user
 *  re-authenticate without leaving chat. After a silent refresh-on-401 fails
 *  (refresh token revoked/expired), this is the path.
 */
export const AuthRequiredBlock: React.FC<{
  provider: string;
  message: string;
}> = ({ provider, message }) => {
  const [state, setState] = React.useState<LoginState>('idle');
  const pollRef = React.useRef<ReturnType<typeof setInterval> | null>(null);
  const flow = LOGIN_FLOWS[provider];

  React.useEffect(() => () => {
    if (pollRef.current) clearInterval(pollRef.current);
  }, []);

  const handleLogin = async () => {
    if (!flow) return;
    setState('waiting');
    try {
      const resp = await fetch(flow.loginUrl, { method: 'POST' });
      // The daemon opens the system browser itself; only pop a fallback tab
      // if it explicitly reports it couldn't.
      const out = await resp.json().catch(() => ({}));
      if (out && out.opened === false && out.url) window.open(out.url, '_blank', 'noopener');
    } catch {
      setState('idle');
      return;
    }
    if (pollRef.current) clearInterval(pollRef.current);
    pollRef.current = setInterval(async () => {
      try {
        const resp = await fetch(flow.statusUrl);
        if (!resp.ok) return;
        const data = await resp.json();
        if (flow.isDone(data)) {
          if (pollRef.current) clearInterval(pollRef.current);
          setState('done');
        }
      } catch { /* keep polling */ }
    }, 2000);
    // Stop polling after the backend's 5-minute login window.
    setTimeout(() => {
      if (pollRef.current) clearInterval(pollRef.current);
      setState(s => (s === 'waiting' ? 'idle' : s));
    }, 300_000);
  };

  return (
    <div className="rounded-lg border border-amber-300 dark:border-amber-700 bg-amber-50 dark:bg-amber-950/40 px-4 py-3 text-sm text-amber-900 dark:text-amber-200">
      <div className="flex items-start gap-2">
        <span className="mt-0.5 shrink-0 text-amber-500 dark:text-amber-400">&#x26A0;</span>
        <div className="space-y-2">
          <MarkdownContent text={message} />
          {flow ? (
            state === 'done' ? (
              <div className="text-green-700 dark:text-green-400">
                Signed in — send your message again.
              </div>
            ) : (
              <button
                onClick={handleLogin}
                disabled={state === 'waiting'}
                className="rounded-md bg-green-600 hover:bg-green-700 disabled:opacity-60 px-3 py-1.5 text-white text-xs font-medium"
              >
                {state === 'waiting' ? 'Waiting for browser login…' : flow.label}
              </button>
            )
          ) : null}
        </div>
      </div>
    </div>
  );
};
