import React, { useCallback, useEffect, useRef, useState } from 'react';
import { ArrowLeft } from 'lucide-react';
import { useNavigate, useLocation } from 'react-router-dom';
import type { AppConfig, ManagementTab } from '../../types';
import { ModelsTab } from '../../components/ModelsTab';
import { AgentsTab } from '../../components/AgentsTab';
import { SkillsTab } from '../../components/SkillsTab';
import { ToolsTab } from '../../components/ToolsTab';
import { GeneralTab } from '../../components/GeneralTab';
import { PhoneTab } from '../../components/PhoneTab';
import { MissionPage } from '../../components/MissionPage';
import { StoragePage } from '../../components/StoragePage';
import { RoomTab } from '../../components/RoomTab';
import { useSessionStore } from '../../stores/sessionStore';
import { useServerStore } from '../../stores/serverStore';
import type { SettingsLocationState } from '../../hooks/useOpenSettings';

const tabs: { key: ManagementTab; label: string }[] = [
  { key: 'general', label: 'General' },
  { key: 'phone', label: 'Phone' },
  { key: 'models', label: 'Models' },
  { key: 'agents', label: 'Agents' },
  { key: 'skills', label: 'Skills' },
  { key: 'tools', label: 'Tools' },
  { key: 'mission', label: 'Mission' },
  { key: 'storage', label: 'Storage' },
  { key: 'room', label: 'Room' },
];

export const SettingsHome: React.FC = () => {
  const navigate = useNavigate();
  const location = useLocation();
  const projectRoot = useSessionStore((s) => s.selectedProjectRoot);
  const missionAgents = useServerStore((s) => s.agents);

  // Tab to pre-select. Comes from route state set by useOpenSettings(tab).
  // Reads on every render so a navigate() to the same URL with a new tab
  // still updates the active tab.
  const requestedTab = (location.state as SettingsLocationState | null)?.tab;
  const [activeTab, setActiveTab] = useState<ManagementTab>(requestedTab || 'general');
  const [config, setConfig] = useState<AppConfig | null>(null);
  const [originalConfig, setOriginalConfig] = useState<AppConfig | null>(null);
  const [saving, setSaving] = useState(false);
  const [error, setError] = useState<string | null>(null);
  const [success, setSuccess] = useState(false);
  const [credsDirty, setCredsDirty] = useState(false);
  const saveCredsRef = useRef<(() => Promise<void>) | null>(null);

  useEffect(() => {
    if (requestedTab) setActiveTab(requestedTab);
  }, [requestedTab]);

  const configDirty = config !== null && originalConfig !== null && JSON.stringify(config) !== JSON.stringify(originalConfig);
  const dirty = configDirty || credsDirty;

  const fetchConfig = useCallback(async () => {
    try {
      const resp = await fetch('/api/config');
      if (!resp.ok) {
        setError('Failed to load config');
        return;
      }
      const data: AppConfig = await resp.json();
      setConfig(data);
      setOriginalConfig(data);
      setError(null);
    } catch (e) {
      setError(`Failed to load config: ${e}`);
    }
  }, []);

  useEffect(() => { fetchConfig(); }, [fetchConfig]);

  const saveConfig = async () => {
    if (!config || saving) return;
    setSaving(true);
    setError(null);
    setSuccess(false);
    try {
      if (configDirty) {
        const resp = await fetch('/api/config', {
          method: 'POST',
          headers: { 'Content-Type': 'application/json' },
          body: JSON.stringify(config),
        });
        if (!resp.ok) {
          const text = await resp.text();
          setError(text || 'Save failed');
          return;
        }
        setOriginalConfig(config);
      }
      if (credsDirty && saveCredsRef.current) {
        await saveCredsRef.current();
      }
      setSuccess(true);
      setTimeout(() => setSuccess(false), 2000);
    } catch (e) {
      setError(`Save failed: ${e}`);
    } finally {
      setSaving(false);
    }
  };

  const onBack = () => {
    const server = useServerStore.getState();
    server.fetchModels();
    server.fetchDefaultModels();
    server.fetchOllamaStatus();
    navigate('/');
  };

  const showSaveButton = activeTab === 'models' || activeTab === 'general' || activeTab === 'phone';

  if (!config) {
    return (
      <div className="flex items-center justify-center h-screen bg-slate-100/70 dark:bg-[#0a0a0a] text-slate-500">
        {error ? <p className="text-red-500">{error}</p> : <p>Loading config...</p>}
      </div>
    );
  }

  return (
    <div className="flex flex-col h-screen bg-slate-100/70 dark:bg-[#0a0a0a] text-slate-900 dark:text-slate-200">
      <header className="border-b border-slate-200 dark:border-white/5 bg-white/90 dark:bg-[#0f0f0f]/90 backdrop-blur-md z-50">
        <div className="flex items-center justify-between px-3 md:px-6 py-2 md:py-3">
          <button onClick={onBack} className="flex items-center gap-1.5 text-xs font-semibold text-slate-600 dark:text-slate-300 hover:text-blue-600 transition-colors shrink-0">
            <ArrowLeft size={16} /> Back
          </button>
          <div className="flex items-center gap-2">
            {error && <span className="text-[11px] text-red-500 max-w-40 truncate">{error}</span>}
            {success && <span className="text-[11px] text-green-500">Saved</span>}
            {showSaveButton && (
              <button
                onClick={saveConfig}
                disabled={!dirty || saving}
                className={`px-3 py-1.5 rounded-lg text-xs font-bold transition-colors ${
                  dirty
                    ? 'bg-blue-600 text-white hover:bg-blue-700 shadow-lg shadow-blue-600/20'
                    : 'bg-slate-200 dark:bg-white/10 text-slate-400 cursor-not-allowed'
                }`}
              >
                {saving ? 'Saving...' : 'Save'}
              </button>
            )}
          </div>
        </div>
        <nav className="flex items-center gap-1 px-3 md:px-6 pb-2 overflow-x-auto">
          {tabs.map((tab) => (
            <button
              key={tab.key}
              onClick={() => setActiveTab(tab.key)}
              className={`px-3 py-1.5 rounded-lg text-xs font-semibold transition-colors whitespace-nowrap shrink-0 ${
                activeTab === tab.key
                  ? 'bg-blue-600/10 text-blue-600 dark:text-blue-400'
                  : 'text-slate-500 hover:text-slate-700 dark:hover:text-slate-300 hover:bg-slate-100 dark:hover:bg-white/5'
              }`}
            >
              {tab.label}
            </button>
          ))}
        </nav>
      </header>

      <div className="flex-1 overflow-hidden">
        {activeTab === 'models' && (
          <div className="h-full overflow-y-auto p-3 md:p-6">
            <div className="max-w-4xl mx-auto">
              <ModelsTab config={config} onChange={setConfig} onCredsDirtyChange={setCredsDirty} saveCredsRef={saveCredsRef} />
            </div>
          </div>
        )}

        {activeTab === 'agents' && <AgentsTab projectRoot={projectRoot} />}

        {activeTab === 'skills' && (
          <div className="h-full overflow-y-auto px-3 md:px-6 py-4 md:py-5">
            <div className="max-w-6xl mx-auto h-full">
              <SkillsTab projectRoot={projectRoot} />
            </div>
          </div>
        )}

        {activeTab === 'tools' && (
          <div className="h-full overflow-y-auto p-3 md:p-6">
            <div className="max-w-4xl mx-auto"><ToolsTab /></div>
          </div>
        )}

        {activeTab === 'general' && (
          <div className="h-full overflow-y-auto p-3 md:p-6">
            <div className="max-w-4xl mx-auto"><GeneralTab config={config} onChange={setConfig} /></div>
          </div>
        )}

        {activeTab === 'phone' && (
          <div className="h-full overflow-y-auto p-3 md:p-6">
            <div className="max-w-4xl mx-auto"><PhoneTab config={config} onChange={setConfig} /></div>
          </div>
        )}

        {activeTab === 'mission' && (
          <MissionPage embedded onBack={onBack} projectRoot={projectRoot} agents={missionAgents ?? []} />
        )}

        {activeTab === 'storage' && <StoragePage embedded onBack={onBack} />}

        {activeTab === 'room' && (
          <div className="h-full overflow-y-auto p-3 md:p-5"><RoomTab /></div>
        )}
      </div>
    </div>
  );
};
