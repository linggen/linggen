import React, { useCallback, useEffect, useRef, useState } from 'react';
import { ArrowLeft } from 'lucide-react';
import type { AgentInfo, AppConfig, ManagementTab } from '../types';
import { ModelsTab } from './ModelsTab';
import { AgentsTab } from './AgentsTab';
import { SkillsTab } from './SkillsTab';
import { ToolsTab } from './ToolsTab';
import { GeneralTab } from './GeneralTab';
import { MissionPage } from './MissionPage';
import { StoragePage } from './StoragePage';

const tabs: { key: ManagementTab; label: string }[] = [
  { key: 'models', label: 'Models' },
  { key: 'agents', label: 'Agents' },
  { key: 'skills', label: 'Skills' },
  { key: 'tools', label: 'Tools' },
  { key: 'general', label: 'General' },
  { key: 'mission', label: 'Mission' },
  { key: 'storage', label: 'Storage' },
];

export const SettingsPage: React.FC<{
  onBack: () => void;
  projectRoot?: string;
  initialTab?: ManagementTab;
  missionAgents?: AgentInfo[];
}> = ({ onBack, projectRoot = '', initialTab, missionAgents }) => {
  const [activeTab, setActiveTab] = useState<ManagementTab>(initialTab || 'models');
  const [config, setConfig] = useState<AppConfig | null>(null);
  const [originalConfig, setOriginalConfig] = useState<AppConfig | null>(null);
  const [saving, setSaving] = useState(false);
  const [error, setError] = useState<string | null>(null);
  const [success, setSuccess] = useState(false);
  const [credsDirty, setCredsDirty] = useState(false);
  const saveCredsRef = useRef<(() => Promise<void>) | null>(null);

  useEffect(() => {
    if (initialTab) setActiveTab(initialTab);
  }, [initialTab]);

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
      // Also save credentials if dirty
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

  // Config-based tabs (Models, General) need save button; Agents/Skills manage their own saving
  const showSaveButton = activeTab === 'models' || activeTab === 'general';

  if (!config) {
    return (
      <div className="flex items-center justify-center h-screen bg-slate-100/70 dark:bg-[#0a0a0a] text-slate-500">
        {error ? <p className="text-red-500">{error}</p> : <p>Loading config...</p>}
      </div>
    );
  }

  return (
    <div className="flex flex-col h-screen bg-slate-100/70 dark:bg-[#0a0a0a] text-slate-900 dark:text-slate-200">
      {/* Top bar */}
      <header className="flex items-center justify-between px-6 py-3 border-b border-slate-200 dark:border-white/5 bg-white/90 dark:bg-[#0f0f0f]/90 backdrop-blur-md z-50">
        <button onClick={onBack} className="flex items-center gap-2 text-xs font-semibold text-slate-600 dark:text-slate-300 hover:text-blue-600 transition-colors">
          <ArrowLeft size={16} /> Back
        </button>

        {/* Tab strip */}
        <nav className="flex items-center gap-1">
          {tabs.map((tab) => (
            <button
              key={tab.key}
              onClick={() => setActiveTab(tab.key)}
              className={`px-3 py-1.5 rounded-lg text-xs font-semibold transition-colors ${
                activeTab === tab.key
                  ? 'bg-blue-600/10 text-blue-600 dark:text-blue-400'
                  : 'text-slate-500 hover:text-slate-700 dark:hover:text-slate-300 hover:bg-slate-100 dark:hover:bg-white/5'
              }`}
            >
              {tab.label}
            </button>
          ))}
        </nav>

        <div className="flex items-center gap-3">
          {error && <span className="text-[10px] text-red-500 max-w-60 truncate">{error}</span>}
          {success && <span className="text-[10px] text-green-500">Saved</span>}
          {showSaveButton && (
            <button
              onClick={saveConfig}
              disabled={!dirty || saving}
              className={`px-4 py-1.5 rounded-lg text-xs font-bold transition-colors ${
                dirty
                  ? 'bg-blue-600 text-white hover:bg-blue-700 shadow-lg shadow-blue-600/20'
                  : 'bg-slate-200 dark:bg-white/10 text-slate-400 cursor-not-allowed'
              }`}
            >
              {saving ? 'Saving...' : 'Save'}
            </button>
          )}
          {!showSaveButton && <div className="w-16" />}
        </div>
      </header>

      {/* Tab content */}
      <div className="flex-1 overflow-hidden">
        {activeTab === 'models' && (
          <div className="h-full overflow-y-auto p-6">
            <div className="max-w-4xl mx-auto">
              <ModelsTab config={config} onChange={setConfig} onCredsDirtyChange={setCredsDirty} saveCredsRef={saveCredsRef} />
            </div>
          </div>
        )}

        {activeTab === 'agents' && (
          <AgentsTab projectRoot={projectRoot} />
        )}

        {activeTab === 'skills' && (
          <div className="h-full overflow-y-auto px-6 py-5">
            <div className="max-w-6xl mx-auto h-full">
              <SkillsTab projectRoot={projectRoot} />
            </div>
          </div>
        )}

        {activeTab === 'tools' && (
          <div className="h-full overflow-y-auto p-6">
            <div className="max-w-4xl mx-auto">
              <ToolsTab />
            </div>
          </div>
        )}

        {activeTab === 'general' && (
          <div className="h-full overflow-y-auto p-6">
            <div className="max-w-4xl mx-auto">
              <GeneralTab config={config} onChange={setConfig} />
            </div>
          </div>
        )}

        {activeTab === 'mission' && (
          <MissionPage
            embedded
            onBack={onBack}
            projectRoot={projectRoot}
            agents={missionAgents ?? []}
          />
        )}

        {activeTab === 'storage' && (
          <StoragePage embedded onBack={onBack} />
        )}
      </div>
    </div>
  );
};
