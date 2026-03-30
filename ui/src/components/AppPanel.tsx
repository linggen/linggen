import React from 'react';
import { X } from 'lucide-react';

export interface AppPanelState {
  skill: string;
  launcher: string;
  url: string;
  title: string;
  width?: number;
  height?: number;
}

/**
 * Full-screen overlay with an iframe for local skill apps.
 * Remote mode opens apps in new tabs via ConnectPage — AppPanel is local only.
 */
export const AppPanel: React.FC<{
  app: AppPanelState;
  onClose: () => void;
}> = ({ app, onClose }) => {
  return (
    <div className="fixed inset-0 z-50 flex flex-col bg-white dark:bg-zinc-900">
      <div className="flex items-center justify-between px-4 py-2 border-b border-slate-200 dark:border-white/10 bg-slate-50 dark:bg-zinc-800/50 shrink-0">
        <div className="flex items-center gap-2">
          <span className="text-sm font-semibold text-slate-700 dark:text-slate-200">{app.title}</span>
          <span className="text-[11px] font-mono text-slate-400 dark:text-slate-500">{app.skill}</span>
        </div>
        <button
          onClick={onClose}
          className="p-1.5 hover:bg-slate-200 dark:hover:bg-white/10 rounded-lg transition-colors text-slate-400 hover:text-slate-600 dark:hover:text-slate-300"
        >
          <X size={16} />
        </button>
      </div>
      <div className="flex-1 min-h-0">
        <iframe
          src={app.url}
          title={app.title}
          style={{ width: '100%', height: '100%', border: 'none' }}
          sandbox="allow-scripts allow-same-origin allow-popups allow-forms"
        />
      </div>
    </div>
  );
};
