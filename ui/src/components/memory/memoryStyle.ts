/** Class strings the Memory tab shares, in the Settings house style. */
export { inputCls, labelCls, sectionCls } from '../settings/kit';

// 44 px tall on a phone (a finger), the compact size from md up.
const btn = 'inline-flex items-center gap-1.5 px-3 py-1.5 min-h-11 md:min-h-0 rounded-lg text-xs font-bold transition-colors disabled:opacity-40 disabled:cursor-not-allowed';
export const primaryBtn = `${btn} bg-blue-600 text-white hover:bg-blue-700`;
export const ghostBtn = `${btn} text-slate-600 dark:text-slate-300 hover:bg-slate-100 dark:hover:bg-white/5`;
export const dangerBtn = `${btn} text-red-600 dark:text-red-400 hover:bg-red-50 dark:hover:bg-red-500/10`;

export const chipCls = 'inline-block px-1.5 py-0.5 rounded text-[10px] font-semibold bg-slate-100 dark:bg-white/5 text-slate-500 dark:text-slate-400';

const TIER_CHIP: Record<string, string> = {
  core: 'bg-amber-100 text-amber-700 dark:bg-amber-500/10 dark:text-amber-400',
  semantic: 'bg-blue-100 text-blue-700 dark:bg-blue-500/10 dark:text-blue-400',
  episodic: 'bg-violet-100 text-violet-700 dark:bg-violet-500/10 dark:text-violet-400',
};
export const tierChipCls = (tier: string | undefined) =>
  `inline-block px-1.5 py-0.5 rounded text-[10px] font-semibold ${TIER_CHIP[tier ?? ''] ?? TIER_CHIP.semantic}`;
