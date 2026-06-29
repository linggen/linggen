/**
 * Native Linggen app shell — the app-host. A thin top bar with a tabview of
 * apps + the selected app's page below (CFO / Sys Doctor style, app_mode).
 *
 * This is NOT the dev console (MainApp) — that stays the web-UI-only page.
 * The desktop shell opens this view (`/?launcher=1`); apps run as kept-mounted
 * iframes so switching preserves each app's state.
 */
import React, { useState, useEffect } from 'react';
import { Settings } from 'lucide-react';
import logoUrl from '../assets/logo.svg';
import { LauncherSettings } from './LauncherSettings';

interface AppSkill {
  name: string;
  app: { launcher: string; entry: string };
}

/** Friendly labels for the known apps; falls back to the raw skill name. */
const LABELS: Record<string, string> = {
  cfo: 'CFO',
  'sys-doctor': 'Sys Doctor',
  pulse: 'Pulse',
  dj: 'DJ',
  'shared-memory': 'Memory',
  'arcade-game': 'Arcade',
  'game-table': 'Games',
  'linggen-guide': 'Guide',
  xbot: 'X',
};
const labelFor = (name: string) => LABELS[name] ?? name;

/** Preferred default app, first one that's installed. */
const PREFERRED_DEFAULT = ['cfo', 'sys-doctor', 'pulse'];

export const LauncherApp: React.FC = () => {
  const [apps, setApps] = useState<AppSkill[]>([]);
  const [activeName, setActiveName] = useState<string | null>(null);
  const [opened, setOpened] = useState<string[]>([]);
  const [settingsOpen, setSettingsOpen] = useState(false);

  // Self-fetch the app list over HTTP so the launcher doesn't depend on the
  // WebRTC page_state timing (the dev console's source).
  useEffect(() => {
    fetch('/api/skills')
      .then((r) => (r.ok ? r.json() : []))
      .then((data) => {
        const list: any[] = Array.isArray(data) ? data : data?.skills ?? [];
        setApps(list.filter((s) => s.app && s.app.launcher === 'web'));
      })
      .catch(() => {});
  }, []);

  // Open the default app once the list arrives.
  useEffect(() => {
    if (activeName || apps.length === 0) return;
    const def = PREFERRED_DEFAULT.find((n) => apps.some((a) => a.name === n)) ?? apps[0].name;
    setActiveName(def);
    setOpened([def]);
  }, [apps, activeName]);

  // The shell's native Settings… menu (and the menubar tray) post
  // {type:'linggen:show-settings'} to the main window — open the merged settings.
  useEffect(() => {
    const onMsg = (e: MessageEvent) => {
      if (e.data && e.data.type === 'linggen:show-settings') setSettingsOpen(true);
    };
    window.addEventListener('message', onMsg);
    return () => window.removeEventListener('message', onMsg);
  }, []);

  const open = (name: string) => {
    setActiveName(name);
    setOpened((prev) => (prev.includes(name) ? prev : [...prev, name]));
  };

  // in_launcher=1 tells the app it's hosted inside the unified launcher (vs a
  // standalone branded app) — apps use it to drop their own settings entry
  // point in favor of the launcher's shared settings.
  const urlFor = (a: AppSkill) => `/apps/${a.name}/${a.app.entry}?app_mode=1&in_launcher=1`;

  return (
    <div className="flex flex-col h-screen bg-white dark:bg-[#0a0a0a] text-slate-900 dark:text-slate-200 overflow-hidden">
      {/* Top bar: brand + app tabview */}
      <header className="flex items-center gap-3 px-3 h-11 shrink-0 border-b border-slate-200 dark:border-white/10 bg-white dark:bg-[#0f0f0f]">
        <div className="flex items-center gap-2 shrink-0">
          <img src={logoUrl} alt="Linggen" className="w-6 h-6" />
          <span className="text-sm font-bold tracking-tight text-slate-900 dark:text-white">Linggen</span>
        </div>
        <div className="flex items-center gap-1 overflow-x-auto">
          {apps.map((a) => {
            const active = a.name === activeName;
            return (
              <button
                key={a.name}
                onClick={() => open(a.name)}
                className={`px-3 h-8 rounded-md text-xs font-medium whitespace-nowrap transition-colors ${
                  active
                    ? 'bg-slate-100 dark:bg-white/10 text-slate-900 dark:text-white'
                    : 'text-slate-500 dark:text-slate-400 hover:bg-slate-50 dark:hover:bg-white/5'
                }`}
              >
                {labelFor(a.name)}
              </button>
            );
          })}
        </div>
        <button
          onClick={() => setSettingsOpen(true)}
          title="Settings"
          className="ml-auto shrink-0 flex items-center justify-center w-8 h-8 rounded-md text-slate-400 hover:bg-slate-100 dark:hover:bg-white/10 hover:text-slate-600 dark:hover:text-slate-200 transition-colors"
        >
          <Settings size={16} />
        </button>
      </header>

      {/* Body: each opened app kept mounted; only the active one is visible. */}
      <div className="flex-1 relative min-h-0">
        {apps.length === 0 && (
          <div className="absolute inset-0 flex items-center justify-center text-sm text-slate-400">
            Loading apps…
          </div>
        )}
        {opened.map((name) => {
          const a = apps.find((x) => x.name === name);
          if (!a) return null;
          return (
            <iframe
              key={name}
              src={urlFor(a)}
              title={labelFor(name)}
              className={activeName === name ? 'absolute inset-0 w-full h-full' : 'hidden'}
              style={{ border: 'none' }}
              sandbox="allow-scripts allow-same-origin allow-popups allow-forms"
            />
          );
        })}
      </div>

      {settingsOpen && <LauncherSettings onClose={() => setSettingsOpen(false)} />}
    </div>
  );
};
