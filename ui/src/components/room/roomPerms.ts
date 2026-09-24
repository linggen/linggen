// Room tool presets and the permission ladder they map onto.
import type { SkillInfoFull } from '../../types';

export const TOOL_PRESETS: Record<string, string[]> = {
  chat: [],
  read: ['WebSearch', 'WebFetch'],
  edit: ['WebSearch', 'WebFetch', 'Read', 'Write', 'Edit', 'Glob', 'Grep', 'Bash'],
};

export const ALL_TOOLS = ['WebSearch', 'WebFetch', 'Read', 'Write', 'Edit', 'Glob', 'Grep', 'Bash'];

/** Permission level hierarchy: chat(0) < read(1) < edit(2) < admin(3). */
export const PERM_LEVEL: Record<string, number> = { chat: 0, read: 1, edit: 2, admin: 3 };

export function presetForTools(tools: string[]): string | null {
  for (const [name, preset] of Object.entries(TOOL_PRESETS)) {
    if (preset.length === tools.length && preset.every(t => tools.includes(t))) return name;
  }
  return null;
}

export function permLevelForTools(tools: string[]): number {
  const preset = presetForTools(tools);
  if (preset) return PERM_LEVEL[preset] ?? 0;
  if (tools.some(t => ['Write', 'Edit', 'Bash'].includes(t))) return PERM_LEVEL.edit;
  if (tools.some(t => ['WebSearch', 'WebFetch', 'Read', 'Glob', 'Grep'].includes(t))) return PERM_LEVEL.read;
  return PERM_LEVEL.chat;
}

export const skillPermLevel = (skill: SkillInfoFull | undefined): number =>
  PERM_LEVEL[skill?.permission?.mode ?? 'chat'] ?? 0;

/** The allowed skills that still fit under `level`. */
export function skillsWithin(names: string[], skills: SkillInfoFull[], level: number): string[] {
  return names.filter(name => skillPermLevel(skills.find(s => s.name === name)) <= level);
}
