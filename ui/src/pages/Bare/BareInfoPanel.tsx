import React from 'react';
import { InfoPanel } from '../../components/InfoPanel';
import { useSessionStore } from '../../stores/sessionStore';
import { useServerStore } from '../../stores/serverStore';
import { useOpenSettings } from '../../hooks/useOpenSettings';
import { recordSkillUsage } from '../../lib/skillUsage';
import type { SkillInfo } from '../../types';

/** Bare /info-panel route — for skill apps to iframe the models/skills
 *  sidebar alone. Transport is owned by the entry Root. */
export const BareInfoPanel: React.FC = () => {
  const projectRoot = useSessionStore((s) => s.selectedProjectRoot);
  const models = useServerStore((s) => s.models);
  const skills = useServerStore((s) => s.skills);
  const agents = useServerStore((s) => s.agents);
  const defaultModels = useServerStore((s) => s.defaultModels);
  const ollamaStatus = useServerStore((s) => s.ollamaStatus);
  const reloadingSkills = useServerStore((s) => s.reloadingSkills);
  const openSettings = useOpenSettings();
  const server = useServerStore.getState();

  const onClickSkill = (skill: SkillInfo) => {
    recordSkillUsage(skill.name);
  };

  return (
    <div className="h-screen w-screen overflow-y-auto bg-white dark:bg-[#0f0f0f] p-3 md:p-4 space-y-3">
      <InfoPanel
        models={models}
        skills={skills}
        agents={agents}
        defaultModels={defaultModels}
        ollamaStatus={ollamaStatus}
        reloadingSkills={reloadingSkills}
        projectRoot={projectRoot}
        onToggleDefault={server.toggleDefaultModel}
        onChangeReasoningEffort={server.setReasoningEffort}
        onReloadSkills={() => server.reloadSkills()}
        onOpenSettings={(tab) => openSettings(tab as any)}
        onClickSkill={onClickSkill}
      />
    </div>
  );
};
