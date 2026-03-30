import React, { useState } from 'react';
import type { PendingAskUser, AskUserAnswer } from '../types';

interface ToolPermissionCardProps {
  pending: PendingAskUser;
  onRespond: (questionId: string, answers: AskUserAnswer[]) => void;
}

export const ToolPermissionCard: React.FC<ToolPermissionCardProps> = ({ pending, onRespond }) => {
  const question = pending.questions[0];
  const [otherActive, setOtherActive] = useState(false);
  const [customText, setCustomText] = useState('');

  if (!question) return null;

  const handleClick = (label: string) => {
    onRespond(pending.questionId, [
      { question_index: 0, selected: [label], custom_text: null },
    ]);
  };

  const handleOtherSubmit = () => {
    const text = customText.trim();
    if (!text) return;
    onRespond(pending.questionId, [
      { question_index: 0, selected: [], custom_text: text },
    ]);
  };

  // Determine if the option is a deny action (session or project-level).
  const isDeny = (label: string) =>
    label === 'Deny' || label === 'Cancel' || label.startsWith('Deny ');

  // Blanket allow options — style them muted (session or project-level).
  const isBlanketOption = (label: string) =>
    label.startsWith('Allow all ');

  // Project-persisted option (allow or deny) — show a save indicator.
  const isPersistedOption = (label: string) =>
    label.includes('for this project');

  // Parse the question text: "Bash npm run build" → tool="Bash", command="npm run build"
  // For non-Bash tools: "Write src/main.rs" → tool="Write", command="src/main.rs"
  const qText = question.question;
  const spaceIdx = qText.indexOf(' ');
  const toolName = spaceIdx > 0 ? qText.slice(0, spaceIdx) : qText;
  const commandText = spaceIdx > 0 ? qText.slice(spaceIdx + 1) : null;

  return (
    <section className="bg-white dark:bg-[#141414] rounded-xl border border-amber-200 dark:border-amber-500/20 shadow-sm flex flex-col overflow-hidden">
      {/* Header with tool name + command on same line */}
      <div className="flex items-start gap-2 px-3 py-2 border-b border-amber-100 dark:border-amber-500/10 bg-amber-50/50 dark:bg-amber-500/5">
        <svg className="w-3.5 h-3.5 mt-0.5 text-amber-500 dark:text-amber-400 shrink-0" fill="none" viewBox="0 0 24 24" stroke="currentColor" strokeWidth={2}>
          <path strokeLinecap="round" strokeLinejoin="round" d="M12 9v2m0 4h.01M21 12a9 9 0 11-18 0 9 9 0 0118 0z" />
        </svg>
        <div className="min-w-0 flex-1">
          <span className="text-[12px] font-bold uppercase tracking-wider text-amber-600 dark:text-amber-400">
            Permission Required
          </span>
          <pre className="mt-1 text-[13px] text-slate-700 dark:text-slate-300 font-mono leading-snug overflow-x-auto whitespace-pre-wrap break-all">
            <span className="font-semibold text-amber-700 dark:text-amber-300">{toolName}</span>{commandText ? ` ${commandText}` : ''}
          </pre>
        </div>
      </div>

      {/* Options */}
      <div className="px-3 py-3 space-y-3">

        {/* Vertical button list */}
        <div className="flex flex-col gap-1.5">
          {question.options.map((opt) => (
            <button
              key={opt.label}
              onClick={() => handleClick(opt.label)}
              className={`text-left text-[12px] px-3 py-1.5 rounded-lg border transition-colors cursor-pointer ${
                isDeny(opt.label)
                  ? 'bg-white text-slate-400 border-slate-200 hover:border-red-300 hover:text-red-500 dark:bg-white/5 dark:text-slate-500 dark:border-white/10 dark:hover:border-red-500/40 dark:hover:text-red-400'
                  : isBlanketOption(opt.label)
                    ? 'bg-white text-slate-400 border-slate-200 hover:border-slate-300 hover:text-slate-500 dark:bg-white/5 dark:text-slate-500 dark:border-white/10 dark:hover:border-slate-400/30 dark:hover:text-slate-400'
                    : 'bg-white text-slate-600 border-slate-200 hover:border-amber-300 hover:text-amber-700 hover:bg-amber-50/50 dark:bg-white/5 dark:text-slate-300 dark:border-white/10 dark:hover:border-amber-500/40 dark:hover:bg-amber-500/5'
              }`}
            >
              <span className="font-medium">{opt.label}</span>
              {isPersistedOption(opt.label) && (
                <svg className="inline-block w-3 h-3 ml-1 -mt-0.5 text-slate-400 dark:text-slate-500" fill="none" viewBox="0 0 24 24" stroke="currentColor" strokeWidth={2}>
                  <path strokeLinecap="round" strokeLinejoin="round" d="M8 7H5a2 2 0 00-2 2v9a2 2 0 002 2h14a2 2 0 002-2V9a2 2 0 00-2-2h-3m-1 4l-3 3m0 0l-3-3m3 3V4" />
                </svg>
              )}
              {opt.description && (
                <span className="ml-2 text-[11px] text-slate-400 dark:text-slate-500">
                  {opt.description}
                </span>
              )}
            </button>
          ))}

          {/* "Other..." toggle button */}
          <button
            onClick={() => setOtherActive(prev => !prev)}
            className={`text-left text-[12px] px-3 py-1.5 rounded-lg border transition-colors cursor-pointer ${
              otherActive
                ? 'bg-amber-50 text-amber-700 border-amber-300 dark:bg-amber-500/10 dark:text-amber-400 dark:border-amber-500/30'
                : 'bg-white text-slate-400 border-dashed border-slate-200 hover:border-amber-300 hover:text-amber-600 dark:bg-transparent dark:text-slate-500 dark:border-white/10 dark:hover:border-amber-500/30 dark:hover:text-amber-400'
            }`}
          >
            <span className="font-medium">Other…</span>
            <span className="ml-2 text-[11px] text-slate-400 dark:text-slate-500">
              Tell the agent what to do instead
            </span>
          </button>
        </div>

        {/* Free-text input when "Other..." is active */}
        {otherActive && (
          <div className="flex gap-1.5">
            <input
              type="text"
              autoFocus
              placeholder="Tell the agent what to do instead…"
              value={customText}
              onChange={e => setCustomText(e.target.value)}
              onKeyDown={e => { if (e.key === 'Enter' && customText.trim()) handleOtherSubmit(); }}
              className="flex-1 text-[12px] px-2.5 py-1.5 rounded-lg border border-slate-200 dark:border-white/10 bg-white dark:bg-white/5 text-slate-700 dark:text-slate-300 placeholder-slate-300 dark:placeholder-slate-600 focus:outline-none focus:border-amber-400 dark:focus:border-amber-500/40"
            />
            <button
              onClick={handleOtherSubmit}
              disabled={!customText.trim()}
              className={`text-[12px] px-3 py-1.5 rounded-lg font-medium transition-colors cursor-pointer ${
                customText.trim()
                  ? 'bg-amber-500 text-white hover:bg-amber-600 dark:bg-amber-600 dark:hover:bg-amber-700'
                  : 'bg-slate-100 text-slate-300 dark:bg-white/5 dark:text-slate-600 cursor-not-allowed'
              }`}
            >
              Send
            </button>
          </div>
        )}
      </div>
    </section>
  );
};
