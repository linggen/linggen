import { useCallback, useEffect, useRef } from 'react';
import { useLocation, useNavigate } from 'react-router-dom';
import type { SettingsLocationState } from '../../hooks/useOpenSettings';
import { watchPageEscape } from '../../lib/dialogEscape.mts';
import { useServerStore } from '../../stores/serverStore';
import { SettingsPanel } from './SettingsPanel';

export function SettingsHome({ launcher }: { launcher: boolean }) {
  const navigate = useNavigate();
  const location = useLocation();
  const state = location.state as SettingsLocationState | null;
  // A host passes route state; links from outside can use ?tab= instead.
  const initial = state?.tab ?? new URLSearchParams(location.search).get('tab');
  const returnToLauncher = launcher && state?.returnToLauncher;
  const onBack = useCallback(() => {
    const server = useServerStore.getState();
    server.fetchModels();
    server.fetchDefaultModels();
    server.fetchOllamaStatus();
    if (returnToLauncher) navigate(-1);
    else navigate('/', { replace: launcher });
  }, [launcher, navigate, returnToLauncher]);

  const page = useRef<HTMLDivElement>(null);
  useEffect(() => { page.current?.focus({ preventScroll: true }); }, []);
  useEffect(() => watchPageEscape(document, onBack), [onBack]);

  return (
    <div ref={page} tabIndex={-1} className="h-screen outline-none">
      <SettingsPanel initial={initial} requestKey={launcher ? location.key : initial ?? ''} canOpenRun={!launcher} onBack={onBack} />
    </div>
  );
}
