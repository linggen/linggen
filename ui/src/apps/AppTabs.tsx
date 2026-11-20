/**
 * The desktop app's tabs: the chat home first, then one tab per app, each
 * app's page kept mounted so switching from one app to another reloads and
 * stops nothing. Shown only inside the desktop shell (`inShell()`,
 * lib/appHost.ts); in a browser an app opens in a new browser tab instead, and
 * this hook does nothing.
 *
 * Also here, because only an app page in front of the person can ask it:
 * Yinyue's bodyless stage while that page says `linggen:companion: hidden`.
 */
import React, { useCallback, useEffect, useRef, useState } from 'react';
import { skillsApi } from '../lib/endpoints';
import { appLabel } from './appLabel.mts';
import type { SkillAppConfig, SkillInfoFull } from '../types';
import { frameAllow } from '../lib/frameAllow.mts';
import { launcherNote, type ListState } from '../lib/launcherNote.mts';
import { setAppHost } from '../lib/appHost';
import { cn } from '../lib/cn';

/** A skill that is a web app. */
type AppSkill = SkillInfoFull & { app: SkillAppConfig };
const isWebApp = (s: SkillInfoFull): s is AppSkill => s.app?.launcher === 'web';

/** Curated menu order — products first; anything else falls to the end
 *  alphabetically (by label) so new apps still appear deterministically. */
const PREFERRED_ORDER = ['cfo', 'apple-shifu', 'mac-shifu', 'sys-doctor', 'pulse', 'dj'];
const orderIndex = (name: string) => {
  const i = PREFERRED_ORDER.indexOf(name);
  return i === -1 ? Number.MAX_SAFE_INTEGER : i;
};

/** The tab last used, across restarts: an app's name, or null for the chat
 *  home (no skill is named null). One versioned blob; skills keep their own. */
const UI_KEY = 'linggen:launcher';
const savedTab = (): string | null => {
  try { return JSON.parse(localStorage.getItem(UI_KEY) ?? '')?.active ?? null; } catch { return null; }
};
const saveTab = (active: string | null) => {
  try { localStorage.setItem(UI_KEY, JSON.stringify({ active, v: 1 })); } catch { /* ignore */ }
};

export type AppTabs = {
  enabled: boolean;
  /** The app in front, or null when the chat home is. */
  active: string | null;
  /** The person sees an app's page now (not the chat home, not settings over it). */
  appInFront: boolean;
  /** The app page in front says it holds no body for Yinyue: stand her nowhere. */
  bodylessStage: boolean;
  tabs: AppSkill[];
  apps: AppSkill[];
  opened: string[];
  views: Record<string, string>;
  listState: ListState;
  show: (name: string | null) => void;
  retry: () => void;
  frames: React.MutableRefObject<Map<string, HTMLIFrameElement>>;
  forgetCompanion: (name: string) => void;
};

export function useAppTabs({ enabled, covered }: { enabled: boolean; covered: boolean }): AppTabs {
  const [apps, setApps] = useState<AppSkill[]>([]);
  const [active, setActive] = useState<string | null>(() => (enabled ? savedTab() : null));
  const [opened, setOpened] = useState<string[]>(() => (active ? [active] : []));
  const [listState, setListState] = useState<ListState>('loading');
  const [listTry, setListTry] = useState(0);
  // The view each app's frame was last asked to land on (a tapped offer's `open`).
  const [views, setViews] = useState<Record<string, string>>({});
  // What each app page last said about the companion's body (skill-spec
  // § App-mode: `linggen:companion`). A page that reloads starts from `shown`.
  const [companion, setCompanion] = useState<Record<string, 'hidden' | 'shown'>>({});
  const frames = useRef(new Map<string, HTMLIFrameElement>());

  const show = useCallback((name: string | null) => {
    setActive(name);
    if (name) setOpened((prev) => (prev.includes(name) ? prev : [...prev, name]));
    saveTab(name);
  }, []);

  const forgetCompanion = useCallback((name: string) => setCompanion((prev) => {
    if (!(name in prev)) return prev;
    const { [name]: _gone, ...rest } = prev;
    return rest;
  }), []);

  // The list over the UI's own calls, not page_state, so tabs do not wait on its timing.
  useEffect(() => {
    if (!enabled) return;
    setListState('loading');
    skillsApi.list()
      .then((data) => {
        const list: SkillInfoFull[] = Array.isArray(data) ? data : [];
        const web = list.filter(isWebApp);
        web.sort((a, b) => orderIndex(a.name) - orderIndex(b.name) || appLabel(a).localeCompare(appLabel(b)));
        setApps(web);
        setListState('ready');
      })
      .catch(() => setListState('error'));
  }, [enabled, listTry]);

  // A remembered app that is no longer installed gives way to the chat home.
  useEffect(() => {
    if (listState === 'ready' && active && !apps.some((a) => a.name === active)) show(null);
  }, [listState, apps, active, show]);

  // `list: false` keeps an unfinished app out of the bar; opened directly, it gets a tab.
  const tabs = apps.filter((a) => a.app.list !== false || opened.includes(a.name));

  const appsRef = useRef(apps);
  appsRef.current = apps;
  useEffect(() => {
    if (!enabled) return;
    setAppHost({
      open: (name, view) => {
        if (!appsRef.current.some((a) => a.name === name)) return false;
        // A launch that names a view reloads that app's frame there: the page
        // reads `?view=` as it loads, and what a view means is its own.
        if (view) setViews((prev) => ({ ...prev, [name]: view }));
        show(name);
        return true;
      },
      home: () => show(null),
    });
    return () => setAppHost(null);
  }, [enabled, show]);

  useEffect(() => {
    if (!enabled) return;
    const onMsg = (e: MessageEvent) => {
      if (e.data?.type !== 'linggen:companion' || e.origin !== window.location.origin) return;
      const name = [...frames.current].find(([, f]) => f.contentWindow === e.source)?.[0];
      const want = e.data.companion === 'hidden' ? 'hidden' : 'shown';
      if (name) setCompanion((prev) => (prev[name] === want ? prev : { ...prev, [name]: want }));
    };
    window.addEventListener('message', onMsg);
    return () => window.removeEventListener('message', onMsg);
  }, [enabled]);

  const appInFront = enabled && !!active && !covered;
  return {
    enabled,
    active,
    appInFront,
    // Honoured only for the page the person sees: not another tab, not under settings.
    bodylessStage: appInFront && active !== null && companion[active] === 'hidden',
    tabs,
    apps,
    opened,
    views,
    listState,
    show,
    retry: () => setListTry((n) => n + 1),
    frames,
    forgetCompanion,
  };
}

const tabClass = (on: boolean) => cn(
  'px-3 h-8 rounded-md text-xs font-medium whitespace-nowrap transition-colors',
  on
    ? 'bg-slate-100 dark:bg-white/10 text-slate-900 dark:text-white'
    : 'text-slate-500 dark:text-slate-400 hover:bg-slate-50 dark:hover:bg-white/5',
);

const ListNote: React.FC<{ t: AppTabs }> = ({ t }) => {
  const note = launcherNote(t.listState, t.apps.length);
  if (!note) return null;
  return (
    <span className="flex items-center gap-2 px-2 text-xs text-slate-400 whitespace-nowrap">
      {note}
      {t.listState === 'error' && (
        <button onClick={t.retry} className="text-blue-600 dark:text-blue-400 hover:underline">Retry</button>
      )}
    </span>
  );
};

/** The tab buttons, for the header: the chat home, then each app. */
export const AppTabBar: React.FC<{ t: AppTabs }> = ({ t }) => (
  <nav className="flex items-center gap-1 min-w-0 overflow-x-auto" aria-label="Tabs">
    <button onClick={() => t.show(null)} className={tabClass(t.active === null)} aria-current={t.active === null ? 'page' : undefined}>
      Chat
    </button>
    {t.tabs.map((a) => (
      <button key={a.name} onClick={() => t.show(a.name)} className={tabClass(a.name === t.active)} aria-current={a.name === t.active ? 'page' : undefined}>
        {appLabel(a)}
      </button>
    ))}
    <ListNote t={t} />
  </nav>
);

/** `in_launcher=1` tells the page it is hosted inside the desktop app: it
 *  drops its own settings entry, and DJ may start sound without a tap. */
const frameUrl = (a: AppSkill, view?: string) =>
  `/apps/${a.name}/${a.app.entry}?app_mode=1&in_launcher=1${view ? `&view=${encodeURIComponent(view)}` : ''}`;

/** Every opened app's page, kept mounted; only the one in front is visible. */
export const AppFrames: React.FC<{ t: AppTabs }> = ({ t }) => {
  if (!t.enabled) return null;
  const waiting = t.active && !t.apps.some((a) => a.name === t.active);
  return (
    <div className={t.active ? 'flex-1 relative min-h-0' : 'hidden'}>
      {waiting && (
        <div className="absolute inset-0 flex items-center justify-center">
          <ListNote t={t} />
        </div>
      )}
      {t.opened.map((name) => {
        const a = t.apps.find((x) => x.name === name);
        if (!a) return null;
        return (
          <iframe
            key={name}
            ref={(f) => { if (f) t.frames.current.set(name, f); else t.frames.current.delete(name); }}
            // The page going away (reload, navigation) takes its word with it.
            onLoad={(e) => {
              try {
                e.currentTarget.contentWindow?.addEventListener('pagehide', () => t.forgetCompanion(name));
              } catch { /* not same-origin: nothing it said was honoured */ }
            }}
            src={frameUrl(a, t.views[name])}
            title={appLabel(a)}
            className={t.active === name ? 'absolute inset-0 w-full h-full' : 'hidden'}
            style={{ border: 'none' }}
            sandbox="allow-scripts allow-same-origin allow-popups allow-forms allow-downloads"
            allow={frameAllow(a.app.allow)}
          />
        );
      })}
    </div>
  );
};
