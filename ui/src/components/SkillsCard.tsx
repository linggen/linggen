import React, { useMemo } from 'react';
import { Wrench } from 'lucide-react';
import type { SkillInfoFull } from '../types';

// ── Skill usage tracking (localStorage) ──

const SKILL_USAGE_KEY = 'linggen:skill-usage';

interface SkillUsageEntry {
  count: number;
  lastUsed: number; // epoch ms
}

function getSkillUsage(): Record<string, SkillUsageEntry> {
  try {
    return JSON.parse(localStorage.getItem(SKILL_USAGE_KEY) || '{}');
  } catch { return {}; }
}

/** Record a skill click. Call this when the user activates a skill. */
export function recordSkillUsage(skillName: string): void {
  const usage = getSkillUsage();
  const entry = usage[skillName] || { count: 0, lastUsed: 0 };
  entry.count += 1;
  entry.lastUsed = Date.now();
  usage[skillName] = entry;
  try { localStorage.setItem(SKILL_USAGE_KEY, JSON.stringify(usage)); } catch { /* quota */ }
}

/** Sort skills: most recently used first, then by usage count, then alphabetical. */
function sortByUsage(skills: SkillInfoFull[]): SkillInfoFull[] {
  const usage = getSkillUsage();
  return [...skills].sort((a, b) => {
    const ua = usage[a.name];
    const ub = usage[b.name];
    // Skills with usage history come first
    if (ua && !ub) return -1;
    if (!ua && ub) return 1;
    if (ua && ub) {
      // Most recently used first
      if (ua.lastUsed !== ub.lastUsed) return ub.lastUsed - ua.lastUsed;
      // Then by usage count
      if (ua.count !== ub.count) return ub.count - ua.count;
    }
    // Alphabetical fallback
    return a.name.localeCompare(b.name);
  });
}

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
      className="bg-slate-50 dark:bg-black/20 px-2.5 py-2 rounded-lg border border-slate-200 dark:border-white/5 cursor-pointer hover:bg-slate-100 dark:hover:bg-white/5 hover:border-slate-300 dark:hover:border-white/10 transition-colors"
    >
      <div className="flex items-center justify-between gap-2">
        <span className="font-mono font-bold text-blue-600 dark:text-blue-400 text-[12px] truncate">
          {displayName}
          {trigger && (
            <span className="text-slate-400 dark:text-slate-500 font-normal"> {trigger}</span>
          )}
        </span>
        <div className="flex items-center gap-1.5">
          {isApp && (
            <span className="text-[10px] font-semibold px-1.5 py-0.5 rounded-full uppercase tracking-wide bg-blue-500/10 text-blue-600 dark:text-blue-400">
              App
            </span>
          )}
          <span
            className={`text-[10px] font-semibold px-1.5 py-0.5 rounded-full uppercase tracking-wide ${
              sourceBadgeCls[sourceType] || sourceBadgeCls.Compat
            }`}
          >
            {sourceLabel}
          </span>
        </div>
      </div>
      {skill.description && (
        <div className="mt-1 text-[11px] text-slate-500 dark:text-slate-400 truncate leading-relaxed">
          {skill.description}
        </div>
      )}
      {skill.tool_defs.length > 0 && (
        <div className="mt-1.5 text-[11px] text-slate-400 dark:text-slate-500 flex items-center gap-1">
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
      <div className="p-4 text-center text-[12px] text-slate-400 italic">
        No skills loaded
      </div>
    );
  }

  const projectSkills = useMemo(() => sortByUsage(skills.filter((s) => !isGlobalOrCompat(s))), [skills]);
  const globalSkills = useMemo(() => sortByUsage(skills.filter(isGlobalOrCompat)), [skills]);
  const projectName = projectRoot ? projectRoot.split('/').filter(Boolean).pop() || 'Project' : 'Project';

  return (
    <div className="flex-1 px-2 py-2 overflow-y-auto text-xs space-y-1.5">
      {globalSkills.map((skill) => (
        <SkillRow key={skill.name} skill={skill} onClick={onClickSkill} />
      ))}
      {projectSkills.length > 0 && (
        <>
          <div className={`text-[10px] font-bold text-slate-400 uppercase tracking-wider px-1 ${globalSkills.length > 0 ? 'mt-3' : ''}`}>{projectName}</div>
          {projectSkills.map((skill) => (
            <SkillRow key={skill.name} skill={skill} projectName={projectName} onClick={onClickSkill} />
          ))}
        </>
      )}
    </div>
  );
};
