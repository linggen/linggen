import React, { useCallback, useEffect, useRef, useState } from 'react';
import { useParams } from 'react-router-dom';
import type { AppConfig } from '../../types';
import { ModelsTab } from '../../components/ModelsTab';
import { AgentsTab } from '../../components/AgentsTab';
import { SkillsTab } from '../../components/SkillsTab';
import { ToolsTab } from '../../components/ToolsTab';
import { GeneralTab } from '../../components/GeneralTab';
import { RoomTab } from '../../components/RoomTab';
import { useSessionStore } from '../../stores/sessionStore';
import { appConfig } from '../../lib/endpoints';
import { apiErrorMessage } from '../../lib/api';

// Only the truly form-shaped sections are exposed via bare routes. Mission
// and Storage are full-page experiences with their own chrome — embedding
// them defeats the "bare" goal, so they're intentionally excluded.
type BareTab = 'models' | 'agents' | 'skills' | 'tools' | 'general' | 'room';

const KNOWN_SECTIONS: ReadonlySet<BareTab> = new Set([
  'models', 'agents', 'skills', 'tools', 'general', 'room',
]);

const NEEDS_CONFIG: ReadonlySet<BareTab> = new Set(['models', 'general']);

/**
 * Bare iframe target for /settings/:section. Renders just the section, no
 * back button or tab strip. Sections that mutate config (Models, General)
 * include a thin save bar; others manage their own persistence.
 *
 * See `linggen-app/doc/architecture.md` and `doc/ui-router-migration.md`.
 */
export const BareSection: React.FC = () => {
  const { section } = useParams<{ section: string }>();
  const projectRoot = useSessionStore((s) => s.selectedProjectRoot);

  const tab = section as BareTab | undefined;
  const isKnown = !!tab && KNOWN_SECTIONS.has(tab);
  const needsConfig = !!tab && NEEDS_CONFIG.has(tab);

  const [config, setConfig] = useState<AppConfig | null>(null);
  const [originalConfig, setOriginalConfig] = useState<AppConfig | null>(null);
  const [saving, setSaving] = useState(false);
  const [error, setError] = useState<string | null>(null);
  const [success, setSuccess] = useState(false);
  const [credsDirty, setCredsDirty] = useState(false);
  const saveCredsRef = useRef<(() => Promise<void>) | null>(null);

  const fetchConfig = useCallback(async () => {
    try {
      const data = await appConfig.get();
      setConfig(data);
      setOriginalConfig(data);
      setError(null);
    } catch (e) {
      setError(`Failed to load config: ${e}`);
    }
  }, []);

  useEffect(() => { if (needsConfig) fetchConfig(); }, [needsConfig, fetchConfig]);

  const configDirty = config !== null && originalConfig !== null && JSON.stringify(config) !== JSON.stringify(originalConfig);
  const dirty = configDirty || credsDirty;

  const saveConfig = async () => {
    if (!config || saving) return;
    setSaving(true);
    setError(null);
    setSuccess(false);
    try {
      if (configDirty) {
        try {
          await appConfig.save(config);
        } catch (e) {
          setError(apiErrorMessage(e) || 'Save failed');
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

  if (!isKnown) {
    return (
      <div className="flex items-center justify-center h-screen bg-slate-100/70 dark:bg-[#0a0a0a] text-slate-500">
        <p>Unknown settings section: <code>{section}</code></p>
      </div>
    );
  }

  if (needsConfig && !config) {
    return (
      <div className="flex items-center justify-center h-screen bg-slate-100/70 dark:bg-[#0a0a0a] text-slate-500">
        {error ? <p className="text-red-500">{error}</p> : <p>Loading config...</p>}
      </div>
    );
  }

  return (
    <div className="flex flex-col h-screen bg-slate-100/70 dark:bg-[#0a0a0a] text-slate-900 dark:text-slate-200">
      {needsConfig && dirty && (
        <div className="flex items-center justify-end gap-2 px-3 md:px-6 py-2 border-b border-slate-200 dark:border-white/5 bg-white/90 dark:bg-[#0f0f0f]/90">
          {error && <span className="text-[11px] text-red-500 max-w-40 truncate">{error}</span>}
          {success && <span className="text-[11px] text-green-500">Saved</span>}
          <button
            onClick={saveConfig}
            disabled={saving}
            className="px-3 py-1 rounded-lg text-xs font-bold bg-blue-600 text-white hover:bg-blue-700 disabled:opacity-50"
          >
            {saving ? 'Saving...' : 'Save'}
          </button>
        </div>
      )}

      <div className="flex-1 overflow-hidden">
        {tab === 'models' && config && (
          <div className="h-full overflow-y-auto p-3 md:p-6">
            <div className="max-w-4xl mx-auto">
              <ModelsTab config={config} onChange={setConfig} onCredsDirtyChange={setCredsDirty} saveCredsRef={saveCredsRef} />
            </div>
          </div>
        )}

        {tab === 'agents' && <AgentsTab projectRoot={projectRoot} />}

        {tab === 'skills' && (
          <div className="h-full overflow-y-auto px-3 md:px-6 py-4 md:py-5">
            <div className="max-w-6xl mx-auto h-full">
              <SkillsTab projectRoot={projectRoot} />
            </div>
          </div>
        )}

        {tab === 'tools' && (
          <div className="h-full overflow-y-auto p-3 md:p-6">
            <div className="max-w-4xl mx-auto"><ToolsTab /></div>
          </div>
        )}

        {tab === 'general' && config && (
          <div className="h-full overflow-y-auto p-3 md:p-6">
            <div className="max-w-4xl mx-auto"><GeneralTab config={config} onChange={setConfig} /></div>
          </div>
        )}

        {tab === 'room' && (
          <div className="h-full overflow-y-auto p-3 md:p-5"><RoomTab /></div>
        )}
      </div>
    </div>
  );
};
