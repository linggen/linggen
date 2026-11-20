import React, { useEffect, useRef, useState } from 'react';
import { confirmKey } from '../lib/dialogKeys.mts';
import { watchTopDialogEscape } from '../lib/dialogEscape.mts';

// The in-page modals behind lib/confirmDialog.

const overlayCls =
  'fixed inset-0 z-[1000] flex items-center justify-center bg-black/40';
const cardCls =
  'w-80 max-w-[90vw] rounded-xl bg-white dark:bg-[#1c1c1c] border border-slate-200 dark:border-white/10 shadow-xl p-4';
const msgCls =
  'text-sm text-slate-800 dark:text-slate-200 whitespace-pre-wrap break-words';
const rowCls = 'flex justify-end gap-2 mt-4';
const cancelCls =
  'px-3 py-1.5 rounded-lg text-xs font-bold text-slate-600 dark:text-slate-300 hover:bg-slate-100 dark:hover:bg-white/5';
const okCls =
  'px-3 py-1.5 rounded-lg text-xs font-bold bg-blue-600 text-white hover:bg-blue-700';
const inputCls =
  'w-full mt-3 px-2.5 py-1.5 rounded-lg text-sm bg-slate-50 dark:bg-white/5 border border-slate-200 dark:border-white/10 outline-none focus:border-blue-500';

export const ConfirmCard: React.FC<{ message: string; onDone: (ok: boolean) => void }> = ({ message, onDone }) => {
  const dialog = useRef<HTMLDivElement>(null);
  useEffect(() => {
    const stopEscape = watchTopDialogEscape(dialog.current!, () => onDone(false));
    const onKey = (e: KeyboardEvent) => {
      const onButton = e.target instanceof HTMLElement && e.target.tagName === 'BUTTON';
      if (confirmKey(e.key, e.isComposing || e.keyCode === 229, onButton) === 'confirm') onDone(true);
    };
    window.addEventListener('keydown', onKey);
    return () => { stopEscape(); window.removeEventListener('keydown', onKey); };
  }, [onDone]);
  return (
    <div className={overlayCls} onClick={() => onDone(false)}>
      <div ref={dialog} className={cardCls} role="alertdialog" aria-modal="true" aria-label={message} onClick={(e) => e.stopPropagation()}>
        <p className={msgCls}>{message}</p>
        <div className={rowCls}>
          <button className={cancelCls} onClick={() => onDone(false)}>Cancel</button>
          <button className={okCls} autoFocus onClick={() => onDone(true)}>OK</button>
        </div>
      </div>
    </div>
  );
};

export const PromptCard: React.FC<{
  message: string;
  initial: string;
  onDone: (value: string | null) => void;
}> = ({ message, initial, onDone }) => {
  const [value, setValue] = useState(initial);
  const inputRef = useRef<HTMLInputElement>(null);
  const dialog = useRef<HTMLDivElement>(null);
  useEffect(() => {
    inputRef.current?.select();
    return watchTopDialogEscape(dialog.current!, () => onDone(null));
  }, [onDone]);
  return (
    <div className={overlayCls} onClick={() => onDone(null)}>
      <div ref={dialog} className={cardCls} role="dialog" aria-modal="true" aria-label={message} onClick={(e) => e.stopPropagation()}>
        <p className={msgCls}>{message}</p>
        <input
          ref={inputRef}
          className={inputCls}
          value={value}
          autoFocus
          onChange={(e) => setValue(e.target.value)}
          onKeyDown={(e) => { if (e.nativeEvent.isComposing || e.keyCode === 229) return; if (e.key === 'Enter') onDone(value); }}
        />
        <div className={rowCls}>
          <button className={cancelCls} onClick={() => onDone(null)}>Cancel</button>
          <button className={okCls} onClick={() => onDone(value)}>OK</button>
        </div>
      </div>
    </div>
  );
};
