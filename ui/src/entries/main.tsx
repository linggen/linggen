/**
 * Single UI entry. Two layers of dispatch:
 *
 * 1. View (top-level user-type discriminator):
 *      URL path /embed | ?mode=compact → EmbedApp
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
import React, { useEffect } from 'react';
import ReactDOM from 'react-dom/client';
import { BrowserRouter, Routes, Route, useLocation } from 'react-router-dom';
import { MainApp } from '../apps/MainApp';
import { EmbedApp } from '../apps/EmbedApp';
import { ConsumerApp } from '../apps/ConsumerApp';
import { PetApp } from '../apps/PetApp';
import { SettingsHome } from '../pages/Settings/SettingsHome';
import { BareSection } from '../pages/Settings/BareSection';
import { MissionEditorPage } from '../pages/Mission/MissionEditorPage';
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

const path = window.location.pathname;
const urlParams = new URLSearchParams(window.location.search);
const isEmbedPath = path === '/embed' || path.startsWith('/embed/')
  || urlParams.get('mode') === 'compact';
// The desktop shell's transparent pet window loads `?pet=1` — render only the
// avatar (PetApp), but still let Root mount the transport so her events flow.
const isPetView = urlParams.get('pet') === '1';

type View = 'main' | 'embed' | 'consumer' | 'pet';

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
    : isEmbedPath
      ? 'embed'
      : currentPage === 'consumer'
        ? 'consumer'
        : 'main';

  useEffect(() => {
    (window as { __LINGGEN_VIEW__?: View }).__LINGGEN_VIEW__ = view;
    sendViewContext();
  }, [view]);

  if (view === 'pet') return <PetApp />;
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
        <Route path="/settings" element={<SettingsHome />} />
        <Route path="/settings/:section" element={<BareSection />} />
        <Route path="/missions/edit" element={<MissionEditorPage />} />
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
