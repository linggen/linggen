import React from 'react';
import { Copy, Eraser, Settings } from 'lucide-react';
import { cn } from '../lib/cn';

export const HeaderBar: React.FC<{
  copyChat: () => void;
  copyChatStatus: 'idle' | 'copied' | 'error';
  clearChat: () => void;
  isRunning: boolean;
  onOpenSettings?: () => void;
}> = ({
  copyChat,
  copyChatStatus,
  clearChat,
  isRunning,
  onOpenSettings,
}) => {
  return (
    <header className="flex items-center justify-between px-6 py-2.5 border-b border-slate-200 dark:border-white/5 bg-white/90 dark:bg-[#0f0f0f]/90 backdrop-blur-md z-50">
      {/* Left: Logo */}
      <div className="flex items-center gap-3">
        <img src="/logo.svg" alt="Linggen" className="w-7 h-7" />
        <h1 className="text-base font-bold tracking-tight text-slate-900 dark:text-white">Linggen Agent</h1>
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
          onClick={clearChat}
          className="p-1.5 hover:bg-red-500/10 hover:text-red-500 rounded-md text-slate-400 transition-colors shrink-0"
          title="Clear Chat"
        >
          <Eraser size={14} />
        </button>
      </div>

      {/* Right: Status + Settings */}
      <div className="flex items-center gap-3 bg-slate-100 dark:bg-white/5 px-3 py-1.5 rounded-full border border-slate-200 dark:border-white/10 shadow-sm">
        <div className="flex items-center gap-2">
          <div className={cn('w-2 h-2 rounded-full', isRunning ? 'bg-green-500 animate-pulse' : 'bg-slate-400')} />
          <span className="text-[10px] font-bold uppercase tracking-widest text-slate-500">{isRunning ? 'Active' : 'Standby'}</span>
        </div>
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
