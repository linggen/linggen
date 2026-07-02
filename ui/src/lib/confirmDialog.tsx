import React, { useEffect, useRef, useState } from 'react';
import { createRoot } from 'react-dom/client';

// Async drop-in for window.confirm()/window.prompt(). The native dialogs are
// unimplemented in the Tauri shell's WKWebView (wry has no
// runJavaScriptConfirmPanel delegate), so confirm() returns false and
// prompt() returns null instantly there — every confirm-gated action becomes
// a silent no-op inside Linggen.app. These render a small in-page modal
// instead and work identically in the browser and the shell.
//
//   if (!(await confirmDialog('Remove this session?'))) return;
//   const name = await promptDialog('New file name', 'untitled.md');

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

const ConfirmCard: React.FC<{ message: string; onDone: (ok: boolean) => void }> = ({ message, onDone }) => {
  useEffect(() => {
    const onKey = (e: KeyboardEvent) => {
      if (e.key === 'Escape') onDone(false);
      if (e.key === 'Enter') onDone(true);
    };
    window.addEventListener('keydown', onKey);
    return () => window.removeEventListener('keydown', onKey);
  }, [onDone]);
  return (
    <div className={overlayCls} onClick={() => onDone(false)}>
      <div className={cardCls} onClick={(e) => e.stopPropagation()}>
        <p className={msgCls}>{message}</p>
        <div className={rowCls}>
          <button className={cancelCls} onClick={() => onDone(false)}>Cancel</button>
          <button className={okCls} autoFocus onClick={() => onDone(true)}>OK</button>
        </div>
      </div>
    </div>
  );
};

const PromptCard: React.FC<{
  message: string;
  initial: string;
  onDone: (value: string | null) => void;
}> = ({ message, initial, onDone }) => {
  const [value, setValue] = useState(initial);
  const inputRef = useRef<HTMLInputElement>(null);
  useEffect(() => {
    inputRef.current?.select();
    const onKey = (e: KeyboardEvent) => {
      if (e.key === 'Escape') onDone(null);
    };
    window.addEventListener('keydown', onKey);
    return () => window.removeEventListener('keydown', onKey);
  }, [onDone]);
  return (
    <div className={overlayCls} onClick={() => onDone(null)}>
      <div className={cardCls} onClick={(e) => e.stopPropagation()}>
        <p className={msgCls}>{message}</p>
        <input
          ref={inputRef}
          className={inputCls}
          value={value}
          autoFocus
          onChange={(e) => setValue(e.target.value)}
          onKeyDown={(e) => { if (e.key === 'Enter') onDone(value); }}
        />
        <div className={rowCls}>
          <button className={cancelCls} onClick={() => onDone(null)}>Cancel</button>
          <button className={okCls} onClick={() => onDone(value)}>OK</button>
        </div>
      </div>
    </div>
  );
};

function mount<T>(render: (done: (v: T) => void) => React.ReactElement): Promise<T> {
  return new Promise((resolve) => {
    const host = document.createElement('div');
    document.body.appendChild(host);
    const root = createRoot(host);
    const done = (v: T) => {
      root.unmount();
      host.remove();
      resolve(v);
    };
    root.render(render(done));
  });
}

export const confirmDialog = (message: string): Promise<boolean> =>
  mount<boolean>((done) => <ConfirmCard message={message} onDone={done} />);

export const promptDialog = (message: string, initial = ''): Promise<string | null> =>
  mount<string | null>((done) => <PromptCard message={message} initial={initial} onDone={done} />);
