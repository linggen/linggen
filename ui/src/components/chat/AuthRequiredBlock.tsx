import React from 'react';
import { MarkdownContent } from './MarkdownContent';

type LoginState = 'idle' | 'waiting' | 'done';

/** Inline sign-in CTA shown when a turn fails because the model's OAuth
 *  session expired. Reuses the same login/poll flow as Settings → Models,
 *  so the user can re-authenticate without leaving chat. After a silent
 *  refresh-on-401 fails (refresh token revoked/expired), this is the path.
 */
export const AuthRequiredBlock: React.FC<{
  provider: string;
  message: string;
}> = ({ provider, message }) => {
  const [state, setState] = React.useState<LoginState>('idle');
  const pollRef = React.useRef<ReturnType<typeof setInterval> | null>(null);

  React.useEffect(() => () => {
    if (pollRef.current) clearInterval(pollRef.current);
  }, []);

  const handleLogin = async () => {
    setState('waiting');
    try {
      await fetch('/api/auth/codex/login', { method: 'POST' });
    } catch {
      setState('idle');
      return;
    }
    if (pollRef.current) clearInterval(pollRef.current);
    pollRef.current = setInterval(async () => {
      try {
        const resp = await fetch('/api/auth/codex/status');
        if (!resp.ok) return;
        const data = await resp.json();
        if (data.authenticated) {
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
          {provider === 'chatgpt' ? (
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
                {state === 'waiting' ? 'Waiting for browser login…' : 'Sign in with ChatGPT'}
              </button>
            )
          ) : null}
        </div>
      </div>
    </div>
  );
};
