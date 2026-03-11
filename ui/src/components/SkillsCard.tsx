import React from 'react';
import { Wrench } from 'lucide-react';
import type { SkillInfoFull } from '../types';

const sourceBadgeCls: Record<string, string> = {
  Global: 'bg-purple-500/10 text-purple-600 dark:text-purple-400',
  Project: 'bg-green-500/10 text-green-600 dark:text-green-400',
  Compat: 'bg-slate-500/10 text-slate-600 dark:text-slate-400',
};

function isGlobalOrCompat(skill: SkillInfoFull): boolean {
  const t = skill.source?.type || 'Global';
  return t === 'Global' || t === 'Compat';
}

function SkillRow({ skill, projectName, onClick }: {
  skill: SkillInfoFull;
  projectName?: string;
  onClick?: (skill: SkillInfoFull) => void;
}) {
  const sourceType = skill.source?.type || 'Global';
  const sourceLabel = sourceType === 'Compat'
    ? (skill.source as { type: string; label?: string })?.label || 'Compat'
    : sourceType === 'Global' ? 'Linggen' : (projectName || 'Project');
  const displayName = `/${skill.name}`;
  const trigger = skill.trigger && skill.trigger !== `/${skill.name}` ? skill.trigger : null;
  const argHint = skill.argument_hint ? ` ${skill.argument_hint}` : '';
  const isApp = !!skill.app;

  return (
    <div
      onClick={() => onClick?.(skill)}
      className="bg-slate-50 dark:bg-black/20 px-3 py-2.5 rounded-xl border border-slate-200 dark:border-white/5 cursor-pointer hover:bg-slate-100 dark:hover:bg-white/5 hover:border-slate-300 dark:hover:border-white/10 transition-colors"
    >
      <div className="flex items-center justify-between gap-2">
        <span className="font-mono font-bold text-blue-600 dark:text-blue-400 text-[11px]">
          {displayName}
          {trigger && (
            <span className="text-slate-400 dark:text-slate-500 font-normal"> {trigger}</span>
          )}
          {argHint && (
            <span className="text-slate-400 dark:text-slate-500 font-normal">{argHint}</span>
          )}
        </span>
        <div className="flex items-center gap-1.5">
          {isApp && (
            <span className="text-[9px] font-semibold px-1.5 py-0.5 rounded-full uppercase tracking-wide bg-blue-500/10 text-blue-600 dark:text-blue-400">
              App
            </span>
          )}
          <span
            className={`text-[9px] font-semibold px-1.5 py-0.5 rounded-full uppercase tracking-wide ${
              sourceBadgeCls[sourceType] || sourceBadgeCls.Compat
            }`}
          >
            {sourceLabel}
          </span>
        </div>
      </div>
      {skill.description && (
        <div className="mt-1 text-[10px] text-slate-500 dark:text-slate-400 truncate leading-relaxed">
          {skill.description}
        </div>
      )}
      {skill.tool_defs.length > 0 && (
        <div className="mt-1.5 text-[10px] text-slate-400 dark:text-slate-500 flex items-center gap-1">
          <Wrench size={9} />
          {skill.tool_defs.length} tool{skill.tool_defs.length !== 1 ? 's' : ''}
        </div>
      )}
    </div>
  );
}

export const SkillsCard: React.FC<{
  skills: SkillInfoFull[];
  projectRoot?: string;
  onClickSkill?: (skill: SkillInfoFull) => void;
}> = ({ skills, projectRoot, onClickSkill }) => {
  if (skills.length === 0) {
    return (
      <div className="p-4 text-center text-[11px] text-slate-400 italic">
        No skills loaded
      </div>
    );
  }

  const appSkills = skills.filter((s) => !!s.app);
  const regularSkills = skills.filter((s) => !s.app);
  const globalSkills = regularSkills.filter(isGlobalOrCompat);
  const projectSkills = regularSkills.filter((s) => !isGlobalOrCompat(s));
  const projectName = projectRoot ? projectRoot.split('/').filter(Boolean).pop() || 'Project' : 'Project';

  return (
    <div className="flex-1 p-4 overflow-y-auto text-xs space-y-2">
      {appSkills.length > 0 && (
        <>
          <div className="text-[9px] font-bold text-slate-400 uppercase tracking-wider px-1">Apps</div>
          {appSkills.map((skill) => (
            <SkillRow key={skill.name} skill={skill} onClick={onClickSkill} />
          ))}
        </>
      )}
      {globalSkills.length > 0 && (
        <>
          <div className={`text-[9px] font-bold text-slate-400 uppercase tracking-wider px-1 ${appSkills.length > 0 ? 'mt-3' : ''}`}>Global</div>
          {globalSkills.map((skill) => (
            <SkillRow key={skill.name} skill={skill} onClick={onClickSkill} />
          ))}
        </>
      )}
      {projectSkills.length > 0 && (
        <>
          <div className={`text-[9px] font-bold text-slate-400 uppercase tracking-wider px-1 ${(globalSkills.length > 0 || appSkills.length > 0) ? 'mt-3' : ''}`}>{projectName}</div>
          {projectSkills.map((skill) => (
            <SkillRow key={skill.name} skill={skill} projectName={projectName} onClick={onClickSkill} />
          ))}
        </>
      )}
    </div>
  );
};
