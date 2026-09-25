/**
 * Single UI entry. Two layers of dispatch:
 *
 * 1. View (top-level user-type discriminator):
 *      URL path /embed | ?mode=compact | ?entry=embed → EmbedApp
 *      currentPage === 'consumer' (from user_info) → ConsumerApp
 *      otherwise → main view (MainApp + react-router routes)
 *
 * 2. Routes (within the main view, via react-router-dom):
 *      /settings           → SettingsHome (chromed, overlays MainApp)
 *      /settings/:section  → BareSection (iframe target, no MainApp)
 *      /missions/edit      → MissionEditorPage (overlays MainApp)
 *      /chat | /sessions | /info-panel → bare iframe targets
 *      everything else     → MainApp shell only
 *
 * The `view` signal is pushed to the server whenever it changes; each App
 * notifies on session/project changes within its own view.
 */
import React, { Suspense, lazy, useEffect } from 'react';
import ReactDOM from 'react-dom/client';
import { BrowserRouter, Routes, Route, useLocation } from 'react-router-dom';
import { MainApp } from '../apps/MainApp';
import { EmbedApp } from '../apps/EmbedApp';
import { ConsumerApp } from '../apps/ConsumerApp';
import { BareChat } from '../pages/Bare/BareChat';
import { BareSessions } from '../pages/Bare/BareSessions';
import { BareInfoPanel } from '../pages/Bare/BareInfoPanel';
import { ErrorBoundary } from '../components/ErrorBoundary';
import { useUiStore } from '../stores/uiStore';
import { useSessionStore } from '../stores/sessionStore';
import { useTransport, sendViewContext } from '../hooks/useTransport';
import '../index.css';
import { installFetchProxy } from '../lib/fetchProxy';

installFetchProxy();

// Code-split: surfaces most loads never reach (the pet window, the desktop
// launcher, settings, the mission editor) load on demand, so the chat shell
// doesn't pay for their tabs and editors up front.
const PetApp = lazy(() => import('../apps/PetApp').then((m) => ({ default: m.PetApp })));
const LauncherApp = lazy(() => import('../apps/LauncherApp').then((m) => ({ default: m.LauncherApp })));
const SettingsHome = lazy(() => import('../pages/Settings/SettingsHome').then((m) => ({ default: m.SettingsHome })));
const BareSection = lazy(() => import('../pages/Settings/BareSection').then((m) => ({ default: m.BareSection })));
const MissionEditorPage = lazy(() => import('../pages/Mission/MissionEditorPage').then((m) => ({ default: m.MissionEditorPage })));

/** Full-screen placeholder with the settings/editor page background, so the
 *  chunk load shows the same empty page rather than a flash of nothing. */
const PageFallback: React.FC = () => (
  <div className="h-screen bg-slate-100/70 dark:bg-[#0a0a0a]" />
);

const path = window.location.pathname;
const urlParams = new URLSearchParams(window.location.search);
// Remotely the chat is booted by the relay's connect page, at
// /app/connect/<id>; the skill page's chat bridge marks it `entry=embed`.
const isEmbedPath = path === '/embed' || path.startsWith('/embed/')
  || urlParams.get('mode') === 'compact'
  || urlParams.get('entry') === 'embed';
// The desktop shell's transparent pet window loads `?pet=1` — render only the
// avatar (PetApp), but still let Root mount the transport so her events flow.
const isPetView = urlParams.get('pet') === '1';
// The desktop Linggen app opens `?launcher=1` — the app-host (tabview + app
// pages), NOT the dev console (MainApp), which stays web-UI-only.
const isLauncherView = urlParams.get('launcher') === '1';

type View = 'main' | 'embed' | 'consumer' | 'pet' | 'launcher';

const Root: React.FC = () => {
  const currentPage = useUiStore((s) => s.currentPage);
  const sessionId = useSessionStore((s) => s.activeSessionId);
  const location = useLocation();

  // Hoist transport to Root so the WebRTC singleton outlives every route
  // transition. Children (MainApp, EmbedApp, ConsumerApp, bare pages) no
  // longer call useTransport themselves — their unmount on navigation would
  // otherwise tear down the connection.
  useTransport({ sessionId });

  const view: View = isPetView
    ? 'pet'
    : isLauncherView
      ? 'launcher'
      : isEmbedPath
        ? 'embed'
        : currentPage === 'consumer'
          ? 'consumer'
          : 'main';

  // The server scopes what this surface receives by its view context — an
  // embed is pinned to one session — so it follows the active session too: a
  // New chat inside an embed switched the panel while the pin stayed on the
  // old session, and every event of the new one was dropped (2026-09-17).
  useEffect(() => {
    (window as { __LINGGEN_VIEW__?: View }).__LINGGEN_VIEW__ = view;
    sendViewContext();
  }, [view, sessionId]);

  if (view === 'pet') return <Suspense fallback={null}><PetApp /></Suspense>;
  if (view === 'launcher') return <Suspense fallback={null}><LauncherApp /></Suspense>;
  if (view === 'embed') return <EmbedApp />;
  if (view === 'consumer') return <ConsumerApp />;

  // Bare iframe targets: skills load these directly to compose their own
  // frame pages. No main shell — just the bare component. See
  // doc/ui-router-migration.md.
  const isBareRoute =
    location.pathname.startsWith('/settings/') ||
    BARE_PATHS.has(location.pathname);

  return (
    <>
      {!isBareRoute && <MainApp />}
      <Routes>
        <Route path="/settings" element={<Suspense fallback={<PageFallback />}><SettingsHome /></Suspense>} />
        <Route path="/settings/:section" element={<Suspense fallback={<PageFallback />}><BareSection /></Suspense>} />
        <Route path="/missions/edit" element={<Suspense fallback={<PageFallback />}><MissionEditorPage /></Suspense>} />
        <Route path="/chat" element={<BareChat />} />
        <Route path="/sessions" element={<BareSessions />} />
        <Route path="/info-panel" element={<BareInfoPanel />} />
        <Route path="*" element={null} />
      </Routes>
    </>
  );
};

const BARE_PATHS: ReadonlySet<string> = new Set(['/chat', '/sessions', '/info-panel']);

ReactDOM.createRoot(document.getElementById('root')!).render(
  <React.StrictMode>
    <ErrorBoundary>
      <BrowserRouter>
        <Root />
      </BrowserRouter>
    </ErrorBoundary>
  </React.StrictMode>
);
