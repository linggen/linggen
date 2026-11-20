import { useParams } from 'react-router-dom';
import { SettingsSectionBody } from '../../components/settings/sections';
import { useSettingsRequirements } from '../../hooks/useSettingsRequirements';
import { BARE_SECTIONS, sectionAvailable } from '../../lib/settingsSections.mts';
import { useAutoSaveConfig } from './useAutoSaveConfig';
import { SaveStatus } from './SaveStatus';

/** Bare iframe target: the section alone, with save status when needed. */
export function BareSection() {
  const { section } = useParams<{ section: string }>();
  const requirements = useSettingsRequirements();
  const entry = BARE_SECTIONS.find((candidate) => candidate.id === section && sectionAvailable(candidate, requirements));
  const save = useAutoSaveConfig(entry?.needsConfig ?? false);

  return (
    <div className="flex flex-col h-screen bg-slate-100/70 dark:bg-[#0a0a0a] text-slate-900 dark:text-slate-200">
      {entry?.needsConfig && save.config && (save.state !== 'idle' || save.error) && (
        <div className="flex items-center justify-end gap-2 px-3 md:px-6 py-1.5 border-b border-slate-200 dark:border-white/5 bg-white/90 dark:bg-[#0f0f0f]/90">
          <SaveStatus state={save.state} error={save.error} />
        </div>
      )}
      <div className="flex-1 overflow-hidden">
        {entry ? <SettingsSectionBody id={entry.id} save={save} requirements={requirements} /> : (
          <div className="flex items-center justify-center h-full text-slate-500">
            <p>Unknown settings section: <code>{section}</code></p>
          </div>
        )}
      </div>
    </div>
  );
}
