import React from 'react';
import { useParams } from 'react-router-dom';
import { ModelsTab } from '../../components/ModelsTab';
import { AgentsPage } from '../../components/AgentsPage';
import { SessionTab } from '../../components/SessionTab';
import { SkillsTab } from '../../components/SkillsTab';
import { ToolsTab } from '../../components/ToolsTab';
import { GeneralTab } from '../../components/GeneralTab';
import { RoomTab } from '../../components/RoomTab';
import { useSessionStore } from '../../stores/sessionStore';
import { useAutoSaveConfig } from './useAutoSaveConfig';
import { SaveStatus } from './SaveStatus';

// Only the truly form-shaped sections are exposed via bare routes. Mission
// and Storage are full-page experiences with their own chrome — embedding
// them defeats the "bare" goal, so they're intentionally excluded.
type BareTab = 'models' | 'agents' | 'skills' | 'tools' | 'general' | 'session' | 'room';

const KNOWN_SECTIONS: ReadonlySet<BareTab> = new Set([
  'models', 'agents', 'skills', 'tools', 'general', 'session', 'room',
]);

const NEEDS_CONFIG: ReadonlySet<BareTab> = new Set(['models', 'general', 'agents', 'session']);

/**
 * Bare iframe target for /settings/:section. Renders just the section, no
 * back button or tab strip. Sections that mutate config (Models, General,
 * Agents, Session) save themselves (useAutoSaveConfig) and show a thin
 * status bar while they do; others manage their own persistence.
 *
 * See `linggen-app/doc/architecture.md` and `doc/ui-router-migration.md`.
 */
export const BareSection: React.FC = () => {
  const { section } = useParams<{ section: string }>();
  const projectRoot = useSessionStore((s) => s.selectedProjectRoot);

  const tab = section as BareTab | undefined;
  const isKnown = !!tab && KNOWN_SECTIONS.has(tab);
  const needsConfig = !!tab && NEEDS_CONFIG.has(tab);

  const { config, setConfig, setCredsDirty, saveCredsRef, state: saveState, error } = useAutoSaveConfig(needsConfig);

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
      {needsConfig && (saveState !== 'idle' || error) && (
        <div className="flex items-center justify-end gap-2 px-3 md:px-6 py-1.5 border-b border-slate-200 dark:border-white/5 bg-white/90 dark:bg-[#0f0f0f]/90">
          <SaveStatus state={saveState} error={error} />
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

        {tab === 'agents' && config && (
          <div className="h-full overflow-y-auto p-3 md:p-6">
            <div className="max-w-4xl mx-auto"><AgentsPage config={config} onChange={setConfig} projectRoot={projectRoot} /></div>
          </div>
        )}

        {tab === 'session' && config && (
          <div className="h-full overflow-y-auto p-3 md:p-6">
            <div className="max-w-4xl mx-auto"><SessionTab config={config} onChange={setConfig} /></div>
          </div>
        )}

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
