export const sectionCls = 'bg-white dark:bg-white/[0.02] border border-slate-200 dark:border-white/5 rounded-xl p-4 space-y-3';
export const labelCls = 'text-xs font-bold uppercase tracking-wider text-slate-500 dark:text-slate-400';
export const inputCls = 'w-full bg-white dark:bg-[#0a0a0a] border border-slate-200 dark:border-white/10 rounded-lg px-3 py-2 text-sm outline-none focus:ring-1 focus:ring-blue-500/50';
export const btnPrimary = 'px-3 py-1.5 bg-blue-600 hover:bg-blue-700 text-white text-xs font-bold rounded-lg transition-colors disabled:opacity-50';
export const smallSelectCls = 'w-full px-2 py-1.5 bg-white dark:bg-[#0a0a0a] border border-slate-200 dark:border-white/10 rounded text-xs';

export const statusDot = (online: boolean, status?: string) =>
  `w-1.5 h-1.5 rounded-full ${status === 'disabled' ? 'bg-amber-500' : online ? 'bg-green-500' : 'bg-slate-400'}`;
export const onlineDot = (online: boolean) => statusDot(online);
