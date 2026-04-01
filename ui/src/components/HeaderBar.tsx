import React, { useState, useEffect, useRef } from 'react';
import { Copy, Eraser, FileText, LogIn, Menu, Settings, Sparkles } from 'lucide-react';
import { cn } from '../lib/cn';
import { useUiStore } from '../stores/uiStore';
import { useProjectStore } from '../stores/projectStore';
import { useAgentStore } from '../stores/agentStore';
import logoUrl from '../assets/logo.svg';

/** Cached user profile from linggen.dev (fetched once on mount). */
let _userCache: { avatar_url?: string; display_name?: string } | null | undefined;
function fetchUserProfile(setUser: (u: typeof _userCache) => void) {
  _userCache = undefined; // reset
  fetch('/api/user/me')
    .then(r => r.ok ? r.json() : null)
    .then(data => { _userCache = data; setUser(data); })
    .catch(() => { _userCache = null; setUser(null); });
}

const UserAvatar: React.FC = () => {
  const [user, setUser] = useState(_userCache);
  const [menuOpen, setMenuOpen] = useState(false);
  const ref = useRef<HTMLDivElement>(null);

  useEffect(() => {
    if (_userCache !== undefined) { setUser(_userCache); return; }
    fetchUserProfile(setUser);
  }, []);

  // Listen for auth completion from popup
  useEffect(() => {
    const handler = (e: MessageEvent) => {
      if (e.data?.type === 'linggen-auth-done') fetchUserProfile(setUser);
    };
    window.addEventListener('message', handler);
    return () => window.removeEventListener('message', handler);
  }, []);

  // Close menu on outside click
  useEffect(() => {
    if (!menuOpen) return;
    const handler = (e: MouseEvent) => {
      if (ref.current && !ref.current.contains(e.target as Node)) setMenuOpen(false);
    };
    document.addEventListener('mousedown', handler);
    return () => document.removeEventListener('mousedown', handler);
  }, [menuOpen]);

  if (user === undefined) return null; // still loading

  // Not logged in — show login button
  if (!user) {
    return (
      <button
        onClick={() => window.open('/api/auth/login?port=' + window.location.port, '_blank', 'width=500,height=600')}
        className="p-1 hover:text-blue-500 text-slate-500 transition-colors"
        title="Sign in to linggen.dev for remote access"
      >
        <LogIn size={14} />
      </button>
    );
  }

  // Logged in — show avatar with dropdown
  return (
    <div className="relative" ref={ref}>
      <button onClick={() => setMenuOpen(!menuOpen)} title={user.display_name || 'Account'}>
        {user.avatar_url ? (
          <img src={user.avatar_url} alt="" className="w-6 h-6 rounded-full ring-1 ring-slate-200 dark:ring-white/10 hover:ring-blue-400 transition-all" />
        ) : (
          <div className="w-6 h-6 rounded-full bg-blue-500 text-white text-[10px] font-bold flex items-center justify-center">
            {(user.display_name || '?')[0].toUpperCase()}
          </div>
        )}
      </button>
      {menuOpen && (
        <div className="absolute right-0 top-full mt-2 w-48 bg-white dark:bg-[#1a1a1a] border border-slate-200 dark:border-white/10 rounded-lg shadow-lg py-1 z-50">
          <div className="px-3 py-2 border-b border-slate-100 dark:border-white/5">
            <div className="text-xs font-medium text-slate-700 dark:text-slate-300 truncate">{user.display_name}</div>
            <div className="text-[10px] text-slate-400">linggen.dev</div>
          </div>
          <a href="https://linggen.dev/app" target="_blank" rel="noopener noreferrer"
             className="block px-3 py-1.5 text-xs text-slate-600 dark:text-slate-400 hover:bg-slate-50 dark:hover:bg-white/5">
            Dashboard
          </a>
          <button
            onClick={async () => {
              await fetch('/api/auth/logout', { method: 'POST' });
              _userCache = null;
              setUser(null);
              setMenuOpen(false);
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

export const HeaderBar: React.FC<{
  copyChat: () => void;
  copyChatStatus: 'idle' | 'copied' | 'error';
  clearChat: () => void;
  isRunning: boolean;
  onOpenSettings?: () => void;
  onToggleMobileMenu?: () => void;
  onToggleInfoPanel?: () => void;
}> = ({
  copyChat,
  copyChatStatus,
  clearChat,
  isRunning,
  onOpenSettings,
  onToggleMobileMenu,
  onToggleInfoPanel,
}) => {
  const [spStatus, setSpStatus] = useState<'idle' | 'copied' | 'error'>('idle');
  const connectionStatus = useUiStore((s) => s.connectionStatus);
  return (
    <header className="flex items-center justify-between px-4 md:px-6 py-2.5 border-b border-slate-200 dark:border-white/5 bg-white/90 dark:bg-[#0f0f0f]/90 backdrop-blur-md z-50">
      {/* Left: Hamburger (mobile) + Logo */}
      <div className="flex items-center gap-2 md:gap-3">
        {onToggleMobileMenu && (
          <button onClick={onToggleMobileMenu} className="md:hidden p-1 rounded hover:bg-slate-100 dark:hover:bg-white/5 text-slate-500">
            <Menu size={18} />
          </button>
        )}
        <a href="https://linggen.dev" target="_blank" rel="noopener noreferrer" className="flex items-center gap-2 md:gap-3 hover:opacity-80 transition-opacity">
          <img src={logoUrl} alt="Linggen" className="w-6 h-6 md:w-7 md:h-7" />
          <h1 className="text-sm md:text-base font-bold tracking-tight text-slate-900 dark:text-white">Linggen</h1>
        </a>
      </div>

      {/* Center: Chat actions */}
      <div className="flex items-center gap-1">
        <button
          onClick={copyChat}
          className={cn(
            'p-1.5 rounded-md transition-colors text-slate-400 shrink-0',
            copyChatStatus === 'copied'
              ? 'bg-green-500/10 text-green-600'
              : copyChatStatus === 'error'
                ? 'bg-red-500/10 text-red-500'
                : 'hover:bg-slate-100 dark:hover:bg-white/5'
          )}
          title={copyChatStatus === 'copied' ? 'Copied' : copyChatStatus === 'error' ? 'Copy failed' : 'Copy Chat'}
        >
          <Copy size={14} />
        </button>
        <button
          onClick={async () => {
            const projectRoot = useProjectStore.getState().selectedProjectRoot;
            const agentId = useAgentStore.getState().selectedAgent;
            if (!projectRoot) { setSpStatus('error'); setTimeout(() => setSpStatus('idle'), 1500); return; }
            try {
              const url = new URL('/api/chat/system-prompt', window.location.origin);
              url.searchParams.append('project_root', projectRoot);
              url.searchParams.append('agent_id', agentId);
              const resp = await fetch(url.toString());
              if (!resp.ok) { setSpStatus('error'); setTimeout(() => setSpStatus('idle'), 1500); return; }
              const data = await resp.json();
              const text = data.system_prompt;
              if (!text) { setSpStatus('error'); setTimeout(() => setSpStatus('idle'), 1500); return; }
              // Try clipboard API first, fall back to textarea+execCommand.
              try {
                await navigator.clipboard.writeText(text);
              } catch {
                const ta = document.createElement('textarea');
                ta.value = text;
                ta.style.position = 'fixed';
                ta.style.opacity = '0';
                document.body.appendChild(ta);
                ta.select();
                document.execCommand('copy');
                document.body.removeChild(ta);
              }
              setSpStatus('copied');
            } catch (e) {
              console.error('Failed to copy system prompt:', e);
              setSpStatus('error');
            }
            setTimeout(() => setSpStatus('idle'), 1500);
          }}
          className={cn(
            'p-1.5 rounded-md transition-colors text-slate-400 shrink-0',
            spStatus === 'copied'
              ? 'bg-green-500/10 text-green-600'
              : spStatus === 'error'
                ? 'bg-red-500/10 text-red-500'
                : 'hover:bg-slate-100 dark:hover:bg-white/5'
          )}
          title={spStatus === 'copied' ? 'Copied!' : spStatus === 'error' ? 'Failed' : 'Copy System Prompt'}
        >
          <FileText size={14} />
        </button>
        <button
          onClick={clearChat}
          className="p-1.5 hover:bg-red-500/10 hover:text-red-500 rounded-md text-slate-400 transition-colors shrink-0"
          title="Clear Chat"
        >
          <Eraser size={14} />
        </button>
      </div>

      {/* Right: Status + Info + Settings + Avatar */}
      <div className="flex items-center gap-2 bg-slate-100 dark:bg-white/5 px-2.5 py-1.5 rounded-full border border-slate-200 dark:border-white/10 shadow-sm">
        {connectionStatus === 'reconnecting' ? (
          <div className="flex items-center gap-1.5">
            <div className="w-2 h-2 rounded-full bg-amber-500 animate-pulse" />
            <span className="text-[11px] font-bold uppercase tracking-widest text-amber-500">Reconnecting</span>
          </div>
        ) : (
          <div className={cn('w-2 h-2 rounded-full', isRunning ? 'bg-green-500 animate-pulse' : 'bg-slate-400')} title={isRunning ? 'Running' : 'Idle'} />
        )}
        {onToggleInfoPanel && (
          <>
            <div className="w-px h-3 bg-slate-300 dark:bg-white/10" />
            <button
              onClick={onToggleInfoPanel}
              className="p-1 hover:text-purple-500 text-slate-500 transition-colors"
              title="Models & Skills"
            >
              <Sparkles size={14} />
            </button>
          </>
        )}
        {onOpenSettings && (
          <>
            <div className="w-px h-3 bg-slate-300 dark:bg-white/10" />
            <button
              onClick={onOpenSettings}
              className="p-1 hover:text-blue-500 text-slate-500 transition-colors"
              title="Settings"
            >
              <Settings size={14} />
            </button>
          </>
        )}
        <UserAvatar />
      </div>
    </header>
  );
};
