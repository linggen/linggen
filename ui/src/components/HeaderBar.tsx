import React, { useState } from 'react';
import { Copy, Eraser, FileText, Menu, Settings, Sparkles } from 'lucide-react';
import { cn } from '../lib/cn';
import { useUiStore } from '../stores/uiStore';
import { useProjectStore } from '../stores/projectStore';
import { useAgentStore } from '../stores/agentStore';
import logoUrl from '../assets/logo.svg';

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

      {/* Right: Status + Info + Settings */}
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
      </div>
    </header>
  );
};
