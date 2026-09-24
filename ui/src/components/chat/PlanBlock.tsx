import React, { useRef, useState } from 'react';
import { MarkdownContent } from './MarkdownContent';
import type { Plan } from '../../types';


/** Renders a plan block with markdown rendering, inline editing, and approval buttons. */
export const PlanBlock: React.FC<{
  plan: Plan;
  statusColor: Record<string, string>;
  pendingPlanAgentId?: string | null;
  agentContext?: Record<string, { tokens: number; messages: number; tokenLimit?: number }>;
  onApprovePlan?: () => void;
  onRejectPlan?: () => void;
  onEditPlan?: (text: string) => void;
  inputRef?: React.RefObject<HTMLTextAreaElement | null>;
}> = ({ plan, statusColor, onApprovePlan, onRejectPlan, onEditPlan }) => {
  const [editing, setEditing] = useState(false);
  const [editText, setEditText] = useState('');
  const editRef = useRef<HTMLTextAreaElement | null>(null);

  const saveEdit = () => {
    if (onEditPlan && editText.trim()) {
      onEditPlan(editText);
    }
    setEditing(false);
  };

  const cancelEdit = () => {
    setEditing(false);
  };

  return (
    <div className="space-y-2">
      <div className="flex items-center gap-2">
        <span className="font-bold text-blue-500">Plan</span>
        <span className={`text-xs font-semibold px-1.5 py-0.5 rounded ${statusColor[plan.status] || statusColor.planned}`}>
          {plan.status}
        </span>
      </div>
      <div className="text-sm opacity-90">{plan.summary}</div>
      {editing ? (
        <div className="space-y-2">
          <textarea
            ref={editRef}
            value={editText}
            onChange={(e) => setEditText(e.target.value)}
            className="w-full text-sm font-mono bg-slate-50 dark:bg-white/5 rounded-md p-3 border border-blue-400 dark:border-blue-500/50 max-h-96 min-h-[120px] overflow-y-auto resize-y focus:outline-none focus:ring-1 focus:ring-blue-400"
            onKeyDown={(e) => {
              if (e.key === 'Escape') cancelEdit();
              if (e.key === 's' && (e.metaKey || e.ctrlKey)) { e.preventDefault(); saveEdit(); }
            }}
          />
          <div className="flex gap-2">
            <button
              onClick={saveEdit}
              className="px-3 py-1 text-sm font-semibold rounded-md bg-blue-600 text-white hover:bg-blue-700"
            >
              Save
            </button>
            <button
              onClick={cancelEdit}
              className="px-3 py-1 text-sm font-semibold rounded-md border border-slate-300 dark:border-white/10 hover:bg-slate-100 dark:hover:bg-white/5"
            >
              Cancel
            </button>
            <span className="text-xs text-slate-400 self-center">Cmd/Ctrl+S to save, Esc to cancel</span>
          </div>
        </div>
      ) : (
        <div className="text-sm bg-slate-50 dark:bg-white/5 rounded-md p-3 border border-slate-200 dark:border-white/10">
          <MarkdownContent text={plan.plan_text || ''} />
        </div>
      )}
      {Array.isArray(plan.items) && plan.items.length > 0 && !editing && (
        <div className="space-y-1">
          <div className="text-sm font-semibold text-slate-600 dark:text-slate-300">Task List</div>
          {plan.items.map((item) => (
            <div key={item.id} className="flex items-start gap-2 text-sm">
              <span className={`mt-0.5 shrink-0 ${item.status === 'done' ? 'text-emerald-500' : item.status === 'in_progress' ? 'text-blue-500' : 'text-slate-400'}`}>
                {item.status === 'done' ? '\u2611' : '\u2610'}
              </span>
              <span className={item.status === 'done' ? 'line-through opacity-60' : ''}>{item.title}</span>
            </div>
          ))}
        </div>
      )}
      {plan.status === 'planned' && onApprovePlan && onRejectPlan && !editing && (
        <div className="flex flex-wrap gap-2 pt-1">
          <button
            onClick={() => onApprovePlan()}
            className="px-3 py-1 text-sm font-semibold rounded-md bg-emerald-600 text-white hover:bg-emerald-700"
          >
            Start building
          </button>
          <button
            onClick={onRejectPlan}
            className="px-3 py-1 text-sm font-semibold rounded-md border border-red-200 dark:border-red-500/20 text-red-600 dark:text-red-400 hover:bg-red-50 dark:hover:bg-red-900/20"
          >
            Reject
          </button>
        </div>
      )}
    </div>
  );
};
