import React, { useEffect } from 'react';

/** The frame the Memory dialogs share: a dimmed overlay and one card.
 *  Escape or a click outside closes it. */
export const ModalShell: React.FC<{ title: string; onClose: () => void; children: React.ReactNode }> = ({ title, onClose, children }) => {
  useEffect(() => {
    const onKey = (e: KeyboardEvent) => { if (e.key === 'Escape') onClose(); };
    window.addEventListener('keydown', onKey);
    return () => window.removeEventListener('keydown', onKey);
  }, [onClose]);
  return (
    <div className="fixed inset-0 z-[900] flex items-center justify-center bg-black/40 p-3" onClick={onClose}>
      <div
        role="dialog"
        aria-modal="true"
        aria-label={title}
        onClick={(e) => e.stopPropagation()}
        className="w-full max-w-lg max-h-[90vh] overflow-y-auto rounded-xl bg-white dark:bg-[#1c1c1c] border border-slate-200 dark:border-white/10 shadow-xl p-5 space-y-4"
      >
        <h2 className="text-xs font-bold uppercase tracking-wider text-slate-700 dark:text-slate-300">{title}</h2>
        {children}
      </div>
    </div>
  );
};
