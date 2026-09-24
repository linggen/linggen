/**
 * Models + Skills info panel — shared between desktop right sidebar and mobile drawer.
 */
import React from 'react';
import { RefreshCw, Settings, Sparkles, Zap } from 'lucide-react';
import { CollapsibleCard } from './CollapsibleCard';
import { ModelsCard } from './ModelsCard';
import { SkillsCard } from './SkillsCard';
import type { AgentInfo, ManagementTab, ModelInfo, OllamaPsResponse, SkillInfo } from '../types';

export interface InfoPanelProps {
  models: ModelInfo[];
  skills: SkillInfo[];
  agents: AgentInfo[];
  activeModelId?: string;
  defaultModels: string[];
  ollamaStatus: OllamaPsResponse | null;
  reloadingSkills: boolean;
  projectRoot: string;
  onToggleDefault: (id: string) => void;
  onChangeReasoningEffort: (modelId: string, effort: string | null) => void;
  onReloadSkills: () => void;
  onOpenSettings: (tab: ManagementTab) => void;
  onClickSkill: (skill: SkillInfo) => void;
}

export const InfoPanel: React.FC<InfoPanelProps> = ({
  models, skills, agents, activeModelId,
  defaultModels, ollamaStatus, reloadingSkills,
  projectRoot, onToggleDefault, onChangeReasoningEffort, onReloadSkills,
  onOpenSettings, onClickSkill,
}) => (
  <>
    <CollapsibleCard title="MODELS" icon={<Sparkles size={12} />} iconColor="text-purple-500" badge={`${models.length}`} defaultOpen
      headerAction={
        <button onClick={() => onOpenSettings('models')}
          className="p-1 hover:bg-slate-100 dark:hover:bg-white/5 rounded transition-colors text-slate-400 hover:text-blue-500" title="Manage Models">
          <Settings size={12} />
        </button>
      }>
      <ModelsCard models={models} agents={agents} ollamaStatus={ollamaStatus}
        activeModelId={activeModelId}
        defaultModels={defaultModels} onToggleDefault={onToggleDefault} onChangeReasoningEffort={onChangeReasoningEffort} />
    </CollapsibleCard>
    <CollapsibleCard title="SKILLS" icon={<Zap size={12} />} iconColor="text-amber-500" badge={`${skills.length} loaded`} defaultOpen
      headerAction={
        <div className="flex items-center gap-0.5">
          <button onClick={onReloadSkills} disabled={reloadingSkills}
            className="p-1 hover:bg-slate-100 dark:hover:bg-white/5 rounded transition-colors text-slate-400 hover:text-blue-500 disabled:opacity-50" title="Reload skills from disk">
            <RefreshCw size={12} className={reloadingSkills ? 'animate-spin' : ''} />
          </button>
          <button onClick={() => onOpenSettings('skills')}
            className="p-1 hover:bg-slate-100 dark:hover:bg-white/5 rounded transition-colors text-slate-400 hover:text-blue-500" title="Manage Skills">
            <Settings size={12} />
          </button>
        </div>
      }>
      <SkillsCard skills={skills} projectRoot={projectRoot} onClickSkill={onClickSkill} />
    </CollapsibleCard>
  </>
);
