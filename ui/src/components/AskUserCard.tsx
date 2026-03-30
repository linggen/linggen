import React, { useState } from 'react';
import type { PendingAskUser, AskUserAnswer } from '../types';

interface AskUserCardProps {
  pending: PendingAskUser;
  onRespond: (questionId: string, answers: AskUserAnswer[]) => void;
}

export const AskUserCard: React.FC<AskUserCardProps> = ({ pending, onRespond }) => {
  const { questions } = pending;

  // Track selections per question.
  const [selections, setSelections] = useState<Record<number, Set<string>>>(() => {
    const init: Record<number, Set<string>> = {};
    questions.forEach((_, i) => { init[i] = new Set(); });
    return init;
  });
  const [customTexts, setCustomTexts] = useState<Record<number, string>>(() => {
    const init: Record<number, string> = {};
    questions.forEach((_, i) => { init[i] = ''; });
    return init;
  });
  const [otherActive, setOtherActive] = useState<Record<number, boolean>>(() => {
    const init: Record<number, boolean> = {};
    questions.forEach((_, i) => { init[i] = false; });
    return init;
  });

  // Single question + single-select → auto-submit on click for faster UX
  // (e.g. plan approve/reject, permission prompts).
  const canAutoSubmit = questions.length === 1 && !questions[0].multi_select;

  const toggleOption = (qIdx: number, label: string) => {
    if (canAutoSubmit) {
      // Immediately submit the selection
      onRespond(pending.questionId, [{
        question_index: qIdx,
        selected: [label],
        custom_text: null,
      }]);
      return;
    }
    setSelections(prev => {
      const next = { ...prev };
      const set = new Set(prev[qIdx]);
      const q = questions[qIdx];
      if (q.multi_select) {
        if (set.has(label)) set.delete(label); else set.add(label);
      } else {
        set.clear();
        set.add(label);
      }
      next[qIdx] = set;
      return next;
    });
    // Deactivate "Other" when selecting a regular option (single-select only).
    if (!questions[qIdx].multi_select) {
      setOtherActive(prev => ({ ...prev, [qIdx]: false }));
    }
  };

  const toggleOther = (qIdx: number) => {
    const q = questions[qIdx];
    setOtherActive(prev => {
      const wasActive = prev[qIdx];
      if (!q.multi_select && !wasActive) {
        // Deselect options when switching to Other in single-select.
        setSelections(p => ({ ...p, [qIdx]: new Set() }));
      }
      return { ...prev, [qIdx]: !wasActive };
    });
  };

  const handleSubmit = () => {
    const answers: AskUserAnswer[] = questions.map((_, i) => {
      const selected = Array.from(selections[i]);
      const custom = otherActive[i] && customTexts[i].trim()
        ? customTexts[i].trim()
        : undefined;
      return { question_index: i, selected, custom_text: custom ?? null };
    });
    onRespond(pending.questionId, answers);
  };

  const handleSkip = () => {
    const answers: AskUserAnswer[] = questions.map((_, i) => ({
      question_index: i,
      selected: [],
      custom_text: null,
    }));
    onRespond(pending.questionId, answers);
  };

  // Check if at least one question has a selection or custom text.
  const hasAnyAnswer = questions.some((_, i) =>
    selections[i].size > 0 || (otherActive[i] && customTexts[i].trim())
  );

  return (
    <section className="bg-white dark:bg-[#141414] rounded-xl border border-blue-200 dark:border-blue-500/20 shadow-sm flex flex-col overflow-hidden">
      {/* Header */}
      <div className="flex items-center gap-2 px-3 py-2 border-b border-blue-100 dark:border-blue-500/10 bg-blue-50/50 dark:bg-blue-500/5">
        <span className="text-[12px] font-bold uppercase tracking-wider text-blue-600 dark:text-blue-400">
          Agent needs your input
        </span>
      </div>

      {/* Questions */}
      <div className="px-3 py-3 space-y-4">
        {questions.map((q, qIdx) => (
          <div key={qIdx} className="space-y-2">
            {/* Header chip + question text */}
            <div className="flex items-start gap-2">
              <span className="shrink-0 text-[10px] font-semibold uppercase tracking-wider px-1.5 py-0.5 rounded bg-slate-100 text-slate-500 dark:bg-white/5 dark:text-slate-400">
                {q.header}
              </span>
              <p className="text-[13px] text-slate-700 dark:text-slate-300 leading-snug">
                {q.question}
              </p>
            </div>

            {/* Options */}
            <div className="flex flex-wrap gap-1.5 ml-0.5">
              {q.options.map((opt) => {
                const isSelected = selections[qIdx].has(opt.label);
                return (
                  <button
                    key={opt.label}
                    onClick={() => toggleOption(qIdx, opt.label)}
                    title={opt.description ?? undefined}
                    className={`text-[12px] px-2.5 py-1 rounded-lg border transition-colors cursor-pointer ${
                      isSelected
                        ? 'bg-blue-600 text-white border-blue-600 dark:bg-blue-500 dark:border-blue-500'
                        : 'bg-white text-slate-600 border-slate-200 hover:border-blue-300 hover:text-blue-600 dark:bg-white/5 dark:text-slate-300 dark:border-white/10 dark:hover:border-blue-500/40'
                    }`}
                  >
                    {opt.label}
                  </button>
                );
              })}

              {/* Other button */}
              <button
                onClick={() => toggleOther(qIdx)}
                className={`text-[12px] px-2.5 py-1 rounded-lg border transition-colors cursor-pointer ${
                  otherActive[qIdx]
                    ? 'bg-blue-600 text-white border-blue-600 dark:bg-blue-500 dark:border-blue-500'
                    : 'bg-white text-slate-400 border-dashed border-slate-200 hover:border-blue-300 hover:text-blue-500 dark:bg-transparent dark:text-slate-500 dark:border-white/10 dark:hover:border-blue-500/40'
                }`}
              >
                Other…
              </button>
            </div>

            {/* Option descriptions */}
            {q.options.some(o => o.description) && (
              <div className="ml-0.5 space-y-0.5">
                {q.options.filter(o => selections[qIdx].has(o.label) && o.description).map(o => (
                  <p key={o.label} className="text-[11px] text-slate-400 dark:text-slate-500 italic">
                    {o.label}: {o.description}
                  </p>
                ))}
              </div>
            )}

            {/* Other text input */}
            {otherActive[qIdx] && (
              <input
                type="text"
                autoFocus
                placeholder="Type your answer…"
                value={customTexts[qIdx]}
                onChange={e => setCustomTexts(prev => ({ ...prev, [qIdx]: e.target.value }))}
                className="w-full text-[12px] px-2.5 py-1.5 rounded-lg border border-slate-200 dark:border-white/10 bg-white dark:bg-white/5 text-slate-700 dark:text-slate-300 placeholder-slate-300 dark:placeholder-slate-600 focus:outline-none focus:border-blue-400 dark:focus:border-blue-500/40"
              />
            )}
          </div>
        ))}
      </div>

      {/* Actions */}
      <div className="flex items-center justify-end gap-2 px-3 py-2 border-t border-slate-100 dark:border-white/5">
        <button
          onClick={handleSkip}
          className="text-[12px] px-3 py-1 rounded-lg text-slate-400 hover:text-slate-600 dark:text-slate-500 dark:hover:text-slate-300 cursor-pointer transition-colors"
        >
          Skip
        </button>
        <button
          onClick={handleSubmit}
          disabled={!hasAnyAnswer}
          className={`text-[12px] px-3 py-1 rounded-lg font-medium transition-colors cursor-pointer ${
            hasAnyAnswer
              ? 'bg-blue-600 text-white hover:bg-blue-700 dark:bg-blue-500 dark:hover:bg-blue-600'
              : 'bg-slate-100 text-slate-300 dark:bg-white/5 dark:text-slate-600 cursor-not-allowed'
          }`}
        >
          Submit
        </button>
      </div>
    </section>
  );
};
