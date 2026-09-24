import React from 'react';
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

import { ConfirmCard, PromptCard } from '../components/DialogCards';

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
