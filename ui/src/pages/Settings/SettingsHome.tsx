import { useCallback, useEffect, useRef } from 'react';
import { useLocation, useNavigate } from 'react-router-dom';
import type { SettingsLocationState } from '../../hooks/useOpenSettings';
import { watchPageEscape } from '../../lib/dialogEscape.mts';
import { useServerStore } from '../../stores/serverStore';
import { SettingsPanel } from './SettingsPanel';

export function SettingsHome() {
  const navigate = useNavigate();
  const location = useLocation();
  const state = location.state as SettingsLocationState | null;
  // A host passes route state; links from outside can use ?tab= instead.
  const initial = state?.tab ?? new URLSearchParams(location.search).get('tab');
  const onBack = useCallback(() => {
    const server = useServerStore.getState();
    server.fetchModels();
    server.fetchDefaultModels();
    server.fetchOllamaStatus();
    // The shell under this page stays mounted, so the desktop app's tab in front stays as it was.
    navigate('/');
  }, [navigate]);

  const page = useRef<HTMLDivElement>(null);
  useEffect(() => { page.current?.focus({ preventScroll: true }); }, []);
  useEffect(() => watchPageEscape(document, onBack), [onBack]);

  return (
    <div ref={page} tabIndex={-1} className="h-screen outline-none">
      <SettingsPanel initial={initial} requestKey={initial ?? ''} canOpenRun onBack={onBack} />
    </div>
  );
}
