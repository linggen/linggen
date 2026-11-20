import type { ReactNode } from 'react';
import type { AppConfig } from '../../types';
import type { SettingsSectionId, SettingsRequirements } from '../../lib/settingsSections.mts';
import type { useAutoSaveConfig } from '../../pages/Settings/useAutoSaveConfig';
import { useSessionStore } from '../../stores/sessionStore';
import { GeneralTab } from '../GeneralTab';
import { AgentsPage } from '../AgentsPage';
import { ModelsTab } from '../ModelsTab';
import { SessionTab } from '../SessionTab';
import { SkillsTab } from '../SkillsTab';
import { ToolsTab } from '../ToolsTab';
import { MissionPage } from '../MissionPage';
import { PhoneTab } from '../PhoneTab';
import { RoomTab } from '../RoomTab';
import { MemoryTab } from '../memory/MemoryTab';
import { StoragePage } from '../StoragePage';
import { AccountSection } from './AccountSection';
import { DesktopSection } from './DesktopSection';

type ConfigSave = ReturnType<typeof useAutoSaveConfig>;
type SectionProps = {
  save: ConfigSave;
  projectRoot: string;
  requirements: SettingsRequirements;
  canOpenRun: boolean;
};

function Scroll({ children, wide = false }: { children: ReactNode; wide?: boolean }) {
  return <div className="h-full overflow-y-auto p-3 md:p-6">
    <div className={`${wide ? 'max-w-6xl' : 'max-w-4xl'} mx-auto`}>{children}</div>
  </div>;
}

function configSection(render: (props: SectionProps, config: AppConfig) => ReactNode) {
  return (props: SectionProps) => {
    const { config, error } = props.save;
    if (config) return render(props, config);
    return <div className="flex h-full items-center justify-center text-slate-500">
      {error ? <p className="text-red-500">{error}</p> : <p>Loading config...</p>}
    </div>;
  };
}

const sections = {
  general: configSection(({ save }, config) => <Scroll><GeneralTab config={config} onChange={save.setConfig} /></Scroll>),
  agents: configSection(({ save, projectRoot }, config) => <Scroll><AgentsPage config={config} onChange={save.setConfig} projectRoot={projectRoot} /></Scroll>),
  models: configSection(({ save }, config) => <Scroll><ModelsTab config={config} onChange={save.setConfig} onCredsDirtyChange={save.setCredsDirty} saveCredsRef={save.saveCredsRef} /></Scroll>),
  session: configSection(({ save }, config) => <Scroll><SessionTab config={config} onChange={save.setConfig} /></Scroll>),
  skills: ({ projectRoot }) => <div className="h-full overflow-y-auto px-3 md:px-6 py-4 md:py-5"><div className="max-w-6xl mx-auto h-full"><SkillsTab projectRoot={projectRoot} /></div></div>,
  tools: () => <Scroll><ToolsTab /></Scroll>,
  mission: ({ projectRoot }) => <MissionPage projectRoot={projectRoot} />,
  phone: configSection(({ save }, config) => <Scroll><PhoneTab config={config} onChange={save.setConfig} /></Scroll>),
  room: () => <div className="h-full overflow-y-auto p-3 md:p-5"><RoomTab /></div>,
  memory: ({ canOpenRun }) => <Scroll wide><MemoryTab canOpenRun={canOpenRun} /></Scroll>,
  storage: () => <StoragePage />,
  account: () => <Scroll><AccountSection /></Scroll>,
  desktop: ({ requirements }) => <Scroll><DesktopSection companion={requirements.companion} /></Scroll>,
} satisfies Record<SettingsSectionId, (props: SectionProps) => ReactNode>;

export function SettingsSectionBody({ id, ...props }: Omit<SectionProps, 'projectRoot'> & { id: SettingsSectionId }) {
  const projectRoot = useSessionStore((s) => s.selectedProjectRoot);
  const Section = sections[id];
  return <Section {...props} projectRoot={projectRoot} />;
}
