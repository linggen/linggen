import { ArrowLeft } from 'lucide-react';
import { useLocation, useNavigate } from 'react-router-dom';
import type { SettingsLocationState } from '../../hooks/useOpenSettings';
import { useServerStore } from '../../stores/serverStore';
import { SettingsPanel } from './SettingsPanel';

export function SettingsHome() {
  const navigate = useNavigate();
  const location = useLocation();
  // A host passes route state; links from outside can use ?tab= instead.
  const initial = (location.state as SettingsLocationState | null)?.tab ?? new URLSearchParams(location.search).get('tab');
  const onBack = () => {
    const server = useServerStore.getState();
    server.fetchModels();
    server.fetchDefaultModels();
    server.fetchOllamaStatus();
    navigate('/');
  };

  return (
    <div className="h-screen">
      <SettingsPanel initial={initial} canOpenRun heading={
        <button onClick={onBack} className="flex items-center gap-1.5 text-xs font-semibold text-slate-600 dark:text-slate-300 hover:text-blue-600 transition-colors shrink-0">
          <ArrowLeft size={16} /> Back
        </button>
      } />
    </div>
  );
}
