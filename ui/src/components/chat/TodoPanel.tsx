import React, { useState } from 'react';
import { ChevronDown, ChevronRight, CheckCircle2, Circle, Loader2 } from 'lucide-react';
import type { Plan } from '../../types';

const StatusIcon: React.FC<{ status: string }> = ({ status }) => {
  switch (status) {
    case 'completed':
    case 'done':
      return <CheckCircle2 size={14} className="text-emerald-500 shrink-0" />;
    case 'in_progress':
    case 'working':
      return <Loader2 size={14} className="text-blue-500 shrink-0 animate-spin" />;
    default:
      return <Circle size={14} className="text-slate-400 dark:text-slate-500 shrink-0" />;
  }
};

export const TodoPanel: React.FC<{
  plan: Plan;
}> = ({ plan }) => {
  const [collapsed, setCollapsed] = useState(false);
  const items = plan.items || [];
  if (items.length === 0) return null;

  const completed = items.filter(i => i.status === 'completed' || i.status === 'done').length;
  const total = items.length;

  return (
    <div className="mx-2 mb-1">
      <div className="bg-white dark:bg-[#141414] border border-slate-200 dark:border-white/10 rounded-lg overflow-hidden">
        {/* Header */}
        <button
          onClick={() => setCollapsed(!collapsed)}
          className="w-full flex items-center gap-2 px-3 py-1.5 hover:bg-slate-50 dark:hover:bg-white/5 transition-colors"
        >
          {collapsed ? (
            <ChevronRight size={14} className="text-slate-400 shrink-0" />
          ) : (
            <ChevronDown size={14} className="text-slate-400 shrink-0" />
          )}
          <span className="text-[12px] font-semibold text-slate-700 dark:text-slate-200 truncate">
            {plan.summary}
          </span>
          <span className="text-[11px] text-slate-400 dark:text-slate-500 ml-auto shrink-0">
            {completed}/{total}
          </span>
          {/* Progress bar */}
          <div className="w-16 h-1.5 bg-slate-200 dark:bg-white/10 rounded-full overflow-hidden shrink-0">
            <div
              className="h-full bg-emerald-500 rounded-full transition-all duration-300"
              style={{ width: `${total > 0 ? (completed / total) * 100 : 0}%` }}
            />
          </div>
        </button>

        {/* Items */}
        {!collapsed && (
          <div className="px-3 pb-2 space-y-0.5">
            {items.map((item) => (
              <div
                key={item.id}
                className="flex items-center gap-2 py-0.5"
              >
                <StatusIcon status={item.status} />
                <span className={`text-[12px] ${
                  item.status === 'completed' || item.status === 'done'
                    ? 'text-slate-400 dark:text-slate-500 line-through'
                    : item.status === 'in_progress' || item.status === 'working'
                    ? 'text-blue-600 dark:text-blue-400 font-medium'
                    : 'text-slate-600 dark:text-slate-300'
                }`}>
                  {item.title}
                </span>
              </div>
            ))}
          </div>
        )}
      </div>
    </div>
  );
};
