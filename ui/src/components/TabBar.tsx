/**
 * Top tab bar for the unified Linggen app. Shows open app tabs (Ling chat is
 * the permanent first one) plus a "+" that opens a picker of installed apps.
 * Apps open in app_mode so they run as the branded, proxy-metered surface.
 */
import React, { useState, useRef, useEffect } from 'react';
import { X, Plus } from 'lucide-react';
import { useTabsStore } from '../stores/tabsStore';
import { useServerStore } from '../stores/serverStore';

export const TabBar: React.FC = () => {
  const { tabs, activeTabId, setActiveTab, closeTab, openAppTab } = useTabsStore();
  const skills = useServerStore((s) => s.skills);
  const [pickerOpen, setPickerOpen] = useState(false);
  const pickerRef = useRef<HTMLDivElement>(null);

  // Web-launcher skills are the ones with a self-contained app UI we can host.
  const apps = skills.filter((s: any) => s.app && s.app.launcher === 'web');
  const openSkills = new Set(tabs.filter((t) => t.kind === 'app').map((t) => t.skill));

  useEffect(() => {
    if (!pickerOpen) return;
    const onDoc = (e: MouseEvent) => {
      if (pickerRef.current && !pickerRef.current.contains(e.target as Node)) setPickerOpen(false);
    };
    document.addEventListener('mousedown', onDoc);
    return () => document.removeEventListener('mousedown', onDoc);
  }, [pickerOpen]);

  const openApp = (skill: any) => {
    const url = `/apps/${skill.name}/${skill.app.entry}?app_mode=1`;
    openAppTab(skill.name, skill.name, url);
    setPickerOpen(false);
  };

  // Nothing to switch to and no apps to open — keep the chrome out of the way.
  if (tabs.length <= 1 && apps.length === 0) return null;

  return (
    <div className="flex items-center gap-1 px-2 h-9 shrink-0 border-b border-slate-200 dark:border-white/5 bg-white dark:bg-[#0f0f0f] overflow-x-auto">
      {tabs.map((t) => {
        const active = t.id === activeTabId;
        return (
          <button
            key={t.id}
            onClick={() => setActiveTab(t.id)}
            className={`group flex items-center gap-1.5 px-3 h-7 rounded-md text-xs font-medium whitespace-nowrap transition-colors ${
              active
                ? 'bg-slate-100 dark:bg-white/10 text-slate-900 dark:text-white'
                : 'text-slate-500 dark:text-slate-400 hover:bg-slate-50 dark:hover:bg-white/5'
            }`}
          >
            <span>{t.title}</span>
            {t.kind !== 'chat' && (
              <span
                role="button"
                tabIndex={0}
                onClick={(e) => { e.stopPropagation(); closeTab(t.id); }}
                className="opacity-0 group-hover:opacity-100 hover:text-red-500 transition-opacity"
              >
                <X size={12} />
              </span>
            )}
          </button>
        );
      })}

      <div className="relative" ref={pickerRef}>
        <button
          onClick={() => setPickerOpen((o) => !o)}
          title="Open an app"
          className="flex items-center justify-center w-7 h-7 rounded-md text-slate-400 hover:bg-slate-100 dark:hover:bg-white/10 hover:text-slate-600 dark:hover:text-slate-200 transition-colors"
        >
          <Plus size={15} />
        </button>
        {pickerOpen && (
          <div className="absolute left-0 top-8 z-50 w-48 py-1 rounded-lg border border-slate-200 dark:border-white/10 bg-white dark:bg-[#1a1a1a] shadow-xl">
            {apps.length === 0 && (
              <p className="px-3 py-2 text-xs text-slate-400">No apps installed</p>
            )}
            {apps.map((s: any) => {
              const isOpen = openSkills.has(s.name);
              return (
                <button
                  key={s.name}
                  onClick={() => openApp(s)}
                  className="w-full flex items-center justify-between px-3 py-1.5 text-xs text-left text-slate-700 dark:text-slate-200 hover:bg-slate-100 dark:hover:bg-white/5"
                >
                  <span>{s.name}</span>
                  {isOpen && <span className="text-[10px] text-slate-400">open</span>}
                </button>
              );
            })}
          </div>
        )}
      </div>
    </div>
  );
};
