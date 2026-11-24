import React from 'react';
import { RowMenu, Tag } from '../models/shell';
import { WORDS, actionLabel, noteActions, noteMeta, type Note, type NoteAction } from '../../lib/memorySources.mts';

/** One of Linggen's own notes: its words, and the one menu (⋯ or a long press) of what may be done to it. */
export const NoteRowView: React.FC<{
  note: Note;
  /** A line under the note, e.g. why it is in the review list. */
  extra?: string;
  onAction: (action: NoteAction, note: Note) => void;
}> = ({ note, extra, onAction }) => {
  const row = React.useRef<HTMLLIElement>(null);
  const items = noteActions(note).map((action) => ({
    label: actionLabel(action),
    run: () => onAction(action, note),
    danger: action === 'delete',
  }));
  return (
    <li ref={row} data-testid="memory-note" data-id={note.id} data-void={note.void !== null} data-no-raise={note.no_raise}
      className="flex items-start gap-2 border-t border-slate-100 py-3 first:border-t-0 dark:border-white/5">
      <div className="min-w-0 flex-1">
        <div className="flex flex-wrap items-center gap-1.5">
          <span className={`text-xs font-semibold ${note.void ? 'line-through text-slate-400' : 'text-slate-800 dark:text-slate-200'}`} data-no-i18n>{note.summary}</span>
          {note.void !== null && <Tag tone="amber">{WORDS.voidTag}</Tag>}
          {note.no_raise && <Tag>{WORDS.noRaiseTag}</Tag>}
          {note.voice === 'user' && <Tag tone="blue">{WORDS.userTag}</Tag>}
        </div>
        {note.body && <p className="mt-1 whitespace-pre-wrap text-[11px] text-slate-600 dark:text-slate-300" data-no-i18n>{note.body}</p>}
        <p className="mt-1 text-[10px] text-slate-400" data-no-i18n>{noteMeta(note)}</p>
        {extra && <p className="mt-0.5 text-[10px] text-amber-700 dark:text-amber-400">{extra}</p>}
      </div>
      <RowMenu items={items} rowRef={row} />
    </li>
  );
};
