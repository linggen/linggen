import { useCallback, useEffect, useState } from 'react';
import { account as accountApi, type Account } from '../../lib/endpoints';

export function AccountSection() {
  const [acct, setAcct] = useState<Account | null>(null);
  const load = useCallback(() => {
    accountApi.get().then(setAcct).catch(() => setAcct(null));
  }, []);
  useEffect(() => { load(); }, [load]);

  const signIn = async () => {
    const acc = await accountApi.signIn();
    if (acc) setAcct(acc);
  };
  const signOut = async () => { await accountApi.logout().catch(() => {}); load(); };
  const button = 'shrink-0 rounded-lg px-3 py-1.5 text-xs font-semibold';

  return (
    <section className="space-y-3">
      <h2 className="text-sm font-bold">Account</h2>
      <p className="text-xs text-slate-500">AI features run through your linggen.dev account (free trial included). One account covers every app.</p>
      {acct === null ? (
        <p className="text-xs text-slate-500">Checking sign-in…</p>
      ) : (
        <div className="flex items-center justify-between gap-3 rounded-xl border border-slate-200 dark:border-white/10 p-3 text-xs">
          {acct.signed_in ? (
            <>
              <div>
                <div className="font-semibold">{acct.user_name || 'Signed in'}</div>
                <div className="text-slate-500">{acct.source === 'remote' ? 'Linked via linggen.dev' : 'linggen.dev'}</div>
              </div>
              <button className={`${button} border border-slate-200 dark:border-white/10`} onClick={signOut}>Sign out</button>
            </>
          ) : (
            <>
              <p className="text-slate-500">Not signed in. Deterministic features work offline; AI needs an account.</p>
              <button className={`${button} bg-emerald-600 text-white hover:bg-emerald-700`} onClick={signIn}>Sign in</button>
            </>
          )}
        </div>
      )}
    </section>
  );
}
