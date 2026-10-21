/**
 * Native Linggen app shell — the app-host. A thin top bar with a tabview of
 * apps + the selected app's page below (CFO / Apple Shifu style, app_mode).
 *
 * This is NOT the dev console (MainApp) — that stays the web-UI-only page.
 * The desktop shell opens this view (`/?launcher=1`); apps run as kept-mounted
 * iframes so switching preserves each app's state.
 */
import React, { useState, useEffect, useRef } from 'react';
import { Settings } from 'lucide-react';
import logoUrl from '../assets/logo.svg';
import { LauncherSettings } from './LauncherSettings';
import { AccountAvatar } from '../components/AccountAvatar';
import { WeatherChip } from '../components/WeatherChip';
import { skillsApi } from '../lib/endpoints';
import { appLabel } from './appLabel';
import { useYinyuePresenter } from '../hooks/useTransport';
import type { SkillAppConfig, SkillInfoFull } from '../types';
import { frameAllow } from '../lib/frameAllow.mts';

/** A skill that is an app. */
type AppSkill = SkillInfoFull & { app: SkillAppConfig };
const isApp = (s: SkillInfoFull): s is AppSkill => !!s.app;


/** Curated menu order — products first; anything else falls to the end
 *  alphabetically (by label) so new apps still appear deterministically. */
const PREFERRED_ORDER = ['cfo', 'apple-shifu', 'mac-shifu', 'sys-doctor', 'pulse', 'dj', 'shared-memory'];
const orderIndex = (name: string) => {
  const i = PREFERRED_ORDER.indexOf(name);
  return i === -1 ? Number.MAX_SAFE_INTEGER : i;
};
/** Preferred default app, first one that's installed. */
const PREFERRED_DEFAULT = ['cfo', 'apple-shifu', 'mac-shifu', 'sys-doctor', 'pulse'];

/** Persisted launcher UI state — restores the last active app across
 *  restarts. One versioned blob; skills keep their own `<skill>:ui` blobs. */
const UI_KEY = 'linggen:launcher';
const loadUi = (): { active?: string } => {
  try { return JSON.parse(localStorage.getItem(UI_KEY) ?? '') || {}; } catch { return {}; }
};
const saveUi = (patch: Record<string, unknown>) => {
  try { localStorage.setItem(UI_KEY, JSON.stringify({ ...loadUi(), ...patch, v: 1 })); } catch { /* ignore */ }
};

export const LauncherApp: React.FC = () => {
  const [apps, setApps] = useState<AppSkill[]>([]);
  const [activeName, setActiveName] = useState<string | null>(null);
  const [opened, setOpened] = useState<string[]>([]);
  // true, or the section to open on (the weather chip opens General).
  const [settingsOpen, setSettingsOpen] = useState<boolean | string>(false);
  // What each app page last said about the companion's body (skill-spec
  // § App-mode: `linggen:companion`). Honoured only for the active tab; a
  // page that reloads starts again from `shown`.
  const [companion, setCompanion] = useState<Record<string, 'hidden' | 'shown'>>({});
  const frames = useRef(new Map<string, HTMLIFrameElement>());
  // The active page says "no body here right now": the launcher holds her as a
  // bodyless stage, so the desktop corner draws nothing (it is not the holder).
  // She comes back to the corner when the page says `shown`, or the tab changes.
  const forgetCompanion = (name: string) => setCompanion((prev) => {
    if (!(name in prev)) return prev;
    const { [name]: _gone, ...rest } = prev;
    return rest;
  });
  useYinyuePresenter(!!activeName && companion[activeName] === 'hidden', true);

  // Self-fetch the app list over HTTP so the launcher doesn't depend on the
  // WebRTC page_state timing (the dev console's source).
  useEffect(() => {
    skillsApi.list()
      .then((data) => {
        const list: SkillInfoFull[] = Array.isArray(data) ? data : [];
        // `list: false` is how a skill that is installed but not finished stays
        // out of the tab bar. It still runs when opened directly.
        const web = list.filter(isApp).filter((s) => s.app.launcher === 'web' && s.app.list !== false);
        web.sort((a, b) =>
          orderIndex(a.name) - orderIndex(b.name) ||
          appLabel(a).localeCompare(appLabel(b)));
        setApps(web);
      })
      .catch(() => {});
  }, []);

  // Open the last active app (if still installed) once the list arrives,
  // falling back to the preferred default. Only the active app is mounted
  // at startup; other tabs lazy-mount on click.
  useEffect(() => {
    if (activeName || apps.length === 0) return;
    const saved = loadUi().active;
    const def = (saved && apps.some((a) => a.name === saved) ? saved : null)
      ?? PREFERRED_DEFAULT.find((n) => apps.some((a) => a.name === n))
      ?? apps[0].name;
    setActiveName(def);
    setOpened([def]);
  }, [apps, activeName]);

  // The shell's native Settings… menu (and the menubar tray) post
  // {type:'linggen:show-settings'} to the main window — open the merged settings.
  useEffect(() => {
    const onMsg = (e: MessageEvent) => {
      if (e.data && e.data.type === 'linggen:show-settings') setSettingsOpen(true);
      if (e.data?.type === 'linggen:companion' && e.origin === window.location.origin) {
        const name = [...frames.current].find(([, f]) => f.contentWindow === e.source)?.[0];
        const want = e.data.companion === 'hidden' ? 'hidden' : 'shown';
        if (name) setCompanion((prev) => (prev[name] === want ? prev : { ...prev, [name]: want }));
      }
    };
    window.addEventListener('message', onMsg);
    return () => window.removeEventListener('message', onMsg);
  }, []);

  const open = (name: string) => {
    setActiveName(name);
    setOpened((prev) => (prev.includes(name) ? prev : [...prev, name]));
    saveUi({ active: name });
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
                {appLabel(a)}
              </button>
            );
          })}
        </div>
        <div className="ml-auto shrink-0 flex items-center gap-2">
          {/* Billing account the daemon spends from — always visible.
              Remounts when Settings closes so a sign in/out done there
              reflects immediately. */}
          <WeatherChip onOpen={() => setSettingsOpen('general')} />
          <AccountAvatar key={String(settingsOpen)} onManage={() => setSettingsOpen(true)} />
          <button
            onClick={() => setSettingsOpen(true)}
            title="Settings"
            className="flex items-center justify-center w-8 h-8 rounded-md text-slate-400 hover:bg-slate-100 dark:hover:bg-white/10 hover:text-slate-600 dark:hover:text-slate-200 transition-colors"
          >
            <Settings size={16} />
          </button>
        </div>
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
              ref={(f) => { if (f) frames.current.set(name, f); else frames.current.delete(name); }}
              // The page going away (reload, navigation) takes its word with it.
              onLoad={(e) => {
                try {
                  e.currentTarget.contentWindow?.addEventListener('pagehide', () => forgetCompanion(name));
                } catch { /* not same-origin: nothing it said was honoured */ }
              }}
              src={urlFor(a)}
              title={appLabel(a)}
              className={activeName === name ? 'absolute inset-0 w-full h-full' : 'hidden'}
              style={{ border: 'none' }}
              sandbox="allow-scripts allow-same-origin allow-popups allow-forms"
              allow={frameAllow(a.app.allow)}
            />
          );
        })}
      </div>

      {settingsOpen && <LauncherSettings initial={settingsOpen === true ? undefined : settingsOpen} onClose={() => setSettingsOpen(false)} />}
    </div>
  );
};
