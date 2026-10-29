/** Where the settings' automatic save stands: a quiet word, never a button. */
import React from 'react';
import type { SaveState } from './useAutoSaveConfig';

export const SaveStatus: React.FC<{ state: SaveState; error: string | null }> = ({ state, error }) => {
  if (error) return <span className="text-[11px] text-red-500 max-w-xl text-right truncate" title={error}>{`Not saved: ${error}`}</span>;
  if (state === 'saving') return <span className="text-[11px] text-slate-400">Saving…</span>;
  if (state === 'saved') return <span className="text-[11px] text-green-500">Saved</span>;
  return null;
};
