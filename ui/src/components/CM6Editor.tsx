import React, { Suspense, lazy } from 'react';
import type { CM6EditorProps } from './CM6EditorImpl';

// CodeMirror + its language data is a large chunk; load it on first edit.
const CM6EditorImpl = lazy(() => import('./CM6EditorImpl').then((m) => ({ default: m.CM6EditorImpl })));

/** While the editor chunk loads, show the text as-is in the editor's font so
 *  the pane keeps its height and nothing jumps when CodeMirror mounts. */
const EditorFallback: React.FC<{ value: string }> = ({ value }) => (
  <pre className="m-0 px-2 py-1 font-mono text-[13px] leading-[1.4] whitespace-pre-wrap break-words text-slate-400">
    {value}
  </pre>
);

export const CM6Editor: React.FC<CM6EditorProps> = (props) => (
  <Suspense fallback={<EditorFallback value={props.value} />}>
    <CM6EditorImpl {...props} />
  </Suspense>
);
