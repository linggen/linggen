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
export function sortByUsage(skills: SkillInfoFull[]): SkillInfoFull[] {
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
