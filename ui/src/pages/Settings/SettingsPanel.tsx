import { useEffect, useState } from 'react';
import { ArrowLeft } from 'lucide-react';
import { appLabel } from '../../apps/appLabel';
import { SettingsSectionBody } from '../../components/settings/sections';
import { useSettingsRequirements } from '../../hooks/useSettingsRequirements';
import { skillsApi } from '../../lib/endpoints';
import { translate } from '../../lib/i18n';
import { APPS_GROUP, appHasSettings, SETTINGS_SECTIONS, resolveSettingsTab, sectionAvailable, type SettingsSection } from '../../lib/settingsSections.mts';
import type { SkillInfoFull } from '../../types';
import { SaveStatus } from './SaveStatus';
import { useAutoSaveConfig } from './useAutoSaveConfig';

type Entry = { id: string; label: string; group: string | null; section: SettingsSection } |
  { id: string; label: string; group: typeof APPS_GROUP; skill: string };

function useAppSettings() {
  const [apps, setApps] = useState<Entry[] | null>(null);
  useEffect(() => {
    let mounted = true;
    skillsApi.list().then((data) => {
      if (!mounted) return;
      setApps((Array.isArray(data) ? data as SkillInfoFull[] : [])
        .filter((s) => s.app?.launcher === 'web' && appHasSettings(s.name))
        .map((s) => ({ id: `app-${s.name}`, label: appLabel({ name: s.name, app: s.app ?? undefined }), group: APPS_GROUP, skill: s.name })));
    }).catch(() => { if (mounted) setApps([]); });
    return () => { mounted = false; };
  }, []);
  return apps;
}

function groupEntries(entries: Entry[]) {
  const groups: { label: string | null; entries: Entry[] }[] = [];
  for (const entry of entries) {
    const last = groups.at(-1);
    if (last && last.label === entry.group) last.entries.push(entry);
    else groups.push({ label: entry.group, entries: [entry] });
  }
  return groups;
}

type Props = {
  initial: string | null;
  requestKey: string;
  onBack: () => void;
  canOpenRun: boolean;
};

export function SettingsPanel({ initial, requestKey, onBack, canOpenRun }: Props) {
  const requirements = useSettingsRequirements();
  const [selection, setSelection] = useState(() => ({ requestKey, picked: resolveSettingsTab(initial) }));
  if (selection.requestKey !== requestKey) setSelection({ requestKey, picked: resolveSettingsTab(initial) });
  const { picked } = selection;
  const setPicked = (id: string) => setSelection({ requestKey, picked: id });
  const save = useAutoSaveConfig();
  const apps = useAppSettings();
  const entries: Entry[] = SETTINGS_SECTIONS.filter((section) => sectionAvailable(section, requirements))
    .map((section) => ({ id: section.id, label: section.label, group: section.group, section }));
  entries.push(...(apps ?? []));
  const waitingForApp = apps === null && picked?.startsWith('app-');
  const active = waitingForApp ? undefined : entries.find((entry) => entry.id === picked) ?? entries[0];

  return (
    <div className="flex flex-col h-full min-h-0 bg-slate-100/70 dark:bg-[#0a0a0a] text-slate-900 dark:text-slate-200">
      <header className="shrink-0 border-b border-slate-200 dark:border-white/5 bg-white/90 dark:bg-[#0f0f0f]/90 backdrop-blur-md z-50">
        <div className="flex items-center justify-between px-3 md:px-6 py-2 md:py-3">
          <button onClick={onBack} className="flex items-center gap-1.5 text-xs font-semibold text-slate-600 dark:text-slate-300 hover:text-blue-600 transition-colors shrink-0">
            <ArrowLeft size={16} /> Back
          </button>
          <div className="flex items-center gap-2 min-h-[30px]">
            <SaveStatus state={save.state} error={save.error} />
          </div>
        </div>
        <nav className="md:hidden flex items-center gap-1 px-3 pb-2 overflow-x-auto">
          {entries.map((entry) => <TabButton key={entry.id} entry={entry} active={active === entry} onPick={setPicked} />)}
        </nav>
      </header>
      <div className="flex-1 flex min-h-0">
        <aside className="hidden md:flex flex-col w-48 shrink-0 border-r border-slate-200 dark:border-white/5 bg-white/60 dark:bg-[#0f0f0f]/60 overflow-y-auto py-3 px-2">
          {groupEntries(entries).map((g) => (
            <div key={g.entries[0].id} className="not-first:mt-4">
              {g.label && <div className="px-3 pb-1 text-[10px] font-bold uppercase tracking-wider text-slate-400">{translate(g.label) ?? g.label}</div>}
              {g.entries.map((entry) => <TabButton key={entry.id} entry={entry} active={active === entry} onPick={setPicked} block />)}
            </div>
          ))}
        </aside>
        <div className="flex-1 min-w-0 overflow-hidden">
          {waitingForApp && <p role="status" className="p-4 text-sm text-slate-500">Loading…</p>}
          {active && ('section' in active
            ? <SettingsSectionBody id={active.section.id} save={save} requirements={requirements} canOpenRun={canOpenRun} />
            : <iframe className="block h-full w-full border-0" src={`/apps/${active.skill}/scripts/settings.html?embed=1`} title={active.label} />)}
        </div>
      </div>
    </div>
  );
}

function TabButton({ entry, active, onPick, block }: { entry: Entry; active: boolean; onPick: (id: string) => void; block?: boolean }) {
  return (
    <button
      onClick={() => onPick(entry.id)}
      aria-current={active ? 'true' : undefined}
      className={`${block ? 'w-full text-left' : 'shrink-0'} px-3 py-1.5 rounded-lg text-xs font-semibold transition-colors whitespace-nowrap ${
        active
          ? 'bg-blue-600/10 text-blue-600 dark:text-blue-400'
          : 'text-slate-500 hover:text-slate-700 dark:hover:text-slate-300 hover:bg-slate-100 dark:hover:bg-white/5'
      }`}
    >
      {translate(entry.label) ?? entry.label}
    </button>
  );
}
