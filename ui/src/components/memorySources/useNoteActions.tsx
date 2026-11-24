import React from 'react';
import { ApiError } from '../../lib/api';
import { memorySourcesApi } from '../../lib/endpoints';
import { confirmDialog, promptDialog } from '../../lib/confirmDialog';
import { WORDS, refusalCode, refusalLine, type Note, type NoteAction } from '../../lib/memorySources.mts';
import { EditNoteDialog } from './EditNoteDialog';

/**
 * What the owner does to one of Linggen's own notes. Each action is one call
 * to the engine's writer; after it (or after a refusal that says the list is
 * old) the list is read again, so the page shows what the engine holds.
 */
export function useNoteActions(reload: () => Promise<void> | void) {
  const [editing, setEditing] = React.useState<Note | null>(null);
  const [error, setError] = React.useState<string | null>(null);

  const attempt = React.useCallback(async (change: () => Promise<unknown>) => {
    setError(null);
    try {
      await change();
    } catch (e) {
      setError(refusalLine(e instanceof ApiError ? refusalCode(e.message) : undefined));
    }
    await reload();
  }, [reload]);

  const run = React.useCallback(async (action: NoteAction, note: Note) => {
    const { id, version } = note;
    switch (action) {
      case 'edit':
        setEditing(note);
        return;
      case 'void': {
        const reason = await promptDialog(WORDS.askReason, '');
        if (reason?.trim()) await attempt(() => memorySourcesApi.void(id, reason.trim(), version));
        return;
      }
      case 'unvoid':
        return attempt(() => memorySourcesApi.unvoid(id, version));
      case 'no-raise':
        return attempt(() => memorySourcesApi.noRaise(id, true, version));
      case 'raise':
        return attempt(() => memorySourcesApi.noRaise(id, false, version));
      case 'delete':
        if (await confirmDialog(WORDS.askDelete)) await attempt(() => memorySourcesApi.remove(id));
        return;
    }
  }, [attempt]);

  const dialog = editing && (
    <EditNoteDialog
      note={editing}
      onClose={() => setEditing(null)}
      onSave={async (change) => {
        setEditing(null);
        await attempt(() => memorySourcesApi.edit(editing.id, { ...change, base: editing.version }));
      }}
    />
  );
  return { run, dialog, error };
}
