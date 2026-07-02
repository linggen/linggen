/**
 * Account avatar — the billing identity (`/api/account`, i.e. account.toml),
 * the account every app's LLM spend is metered against. Shown top-right in
 * both the dev console (HeaderBar) and the app launcher so it's always
 * visible which account the daemon is signed into.
 *
 * Signed in: avatar + dropdown (name, Dashboard, optional Account settings,
 * Sign out). Signed out: a Sign in button running the /api/account/login
 * flow (daemon opens the browser; we poll /api/account until it flips).
 */
import React, { useState, useEffect, useRef, useCallback } from 'react';
import { LogIn } from 'lucide-react';

interface AccountInfo {
  signed_in: boolean;
  user_name?: string;
  avatar_url?: string;
}

export const AccountAvatar: React.FC<{
  /** Extra dropdown row (e.g. the launcher opens its Settings overlay). */
  onManage?: () => void;
}> = ({ onManage }) => {
  const [account, setAccount] = useState<AccountInfo | null>(null);
  const [menuOpen, setMenuOpen] = useState(false);
  const [signingIn, setSigningIn] = useState(false);
  const ref = useRef<HTMLDivElement>(null);

  const refresh = useCallback(() => {
    fetch('/api/account')
      .then((r) => (r.ok ? r.json() : null))
      .then((data) => setAccount(data))
      .catch(() => setAccount(null));
  }, []);

  useEffect(() => {
    refresh();
    // Re-check when the tab regains focus — sign in/out may have happened
    // in the Settings overlay or another window meanwhile.
    window.addEventListener('focus', refresh);
    return () => window.removeEventListener('focus', refresh);
  }, [refresh]);

  // Close menu on outside click
  useEffect(() => {
    if (!menuOpen) return;
    const handler = (e: MouseEvent) => {
      if (ref.current && !ref.current.contains(e.target as Node)) setMenuOpen(false);
    };
    document.addEventListener('mousedown', handler);
    return () => document.removeEventListener('mousedown', handler);
  }, [menuOpen]);

  const signIn = async () => {
    setSigningIn(true);
    try {
      const resp = await fetch('/api/account/login', { method: 'POST' });
      const data = await resp.json().catch(() => ({}));
      if (!data.opened && data.url) window.open(data.url, '_blank');
      // Poll until the callback lands (bounded ~2 min).
      for (let i = 0; i < 60; i++) {
        await new Promise((r) => setTimeout(r, 2000));
        const acc = await fetch('/api/account').then((r) => (r.ok ? r.json() : null)).catch(() => null);
        if (acc?.signed_in) { setAccount(acc); break; }
      }
    } finally {
      setSigningIn(false);
    }
  };

  if (account === null) return null; // loading or endpoint unavailable

  if (!account.signed_in) {
    return (
      <button
        onClick={signIn}
        disabled={signingIn}
        className="p-1 hover:text-blue-500 text-slate-500 transition-colors disabled:opacity-50"
        title="Sign in to linggen.dev"
      >
        <LogIn size={14} className={signingIn ? 'animate-pulse' : undefined} />
      </button>
    );
  }

  return (
    <div className="relative" ref={ref}>
      <button onClick={() => setMenuOpen(!menuOpen)} title={account.user_name || 'Account'}
        className="flex items-center">
        {account.avatar_url ? (
          <img src={account.avatar_url} alt=""
            className="w-6 h-6 rounded-full ring-1 ring-slate-200 dark:ring-white/10 hover:ring-blue-400 transition-all" />
        ) : (
          <div className="w-6 h-6 rounded-full bg-blue-500 text-white text-[10px] font-bold flex items-center justify-center">
            {(account.user_name || '?')[0].toUpperCase()}
          </div>
        )}
      </button>
      {menuOpen && (
        <div className="absolute right-0 top-full mt-2 w-48 bg-white dark:bg-[#1a1a1a] border border-slate-200 dark:border-white/10 rounded-lg shadow-lg py-1 z-50">
          <div className="px-3 py-2 border-b border-slate-100 dark:border-white/5">
            <div className="text-xs font-medium text-slate-700 dark:text-slate-300 truncate">{account.user_name}</div>
            <div className="text-[10px] text-slate-400">linggen.dev</div>
          </div>
          <a href="https://linggen.dev/app" target="_blank" rel="noopener noreferrer"
            className="block px-3 py-1.5 text-xs text-slate-600 dark:text-slate-400 hover:bg-slate-50 dark:hover:bg-white/5">
            Dashboard
          </a>
          {onManage && (
            <button
              onClick={() => { setMenuOpen(false); onManage(); }}
              className="w-full text-left px-3 py-1.5 text-xs text-slate-600 dark:text-slate-400 hover:bg-slate-50 dark:hover:bg-white/5"
            >
              Account settings
            </button>
          )}
          <button
            onClick={async () => {
              await fetch('/api/account/logout', { method: 'POST' });
              setMenuOpen(false);
              refresh();
            }}
            className="w-full text-left px-3 py-1.5 text-xs text-red-500 hover:bg-red-50 dark:hover:bg-red-500/10"
          >
            Sign Out
          </button>
        </div>
      )}
    </div>
  );
};
