import React from 'react';
import { ModalShell } from '../memory/ModalShell';
import { inputCls } from '../settings/kit';
import { WORDS, type Note } from '../../lib/memorySources.mts';

const SUMMARY_MAX = 80;

/** One note's summary and details, changed together. */
export const EditNoteDialog: React.FC<{
  note: Note;
  onClose: () => void;
  onSave: (change: { summary: string; body: string }) => void;
}> = ({ note, onClose, onSave }) => {
  const [summary, setSummary] = React.useState(note.summary);
  const [body, setBody] = React.useState(note.body);
  const changed = summary !== note.summary || body !== note.body;
  return (
    <ModalShell title={WORDS.edit.title} onClose={onClose}>
      <label className="block text-[11px] font-bold uppercase tracking-wider text-slate-500">
        {WORDS.edit.summary}
        <input className={`${inputCls} mt-1 normal-case font-normal`} value={summary} maxLength={SUMMARY_MAX} data-no-i18n
          onChange={(e) => setSummary(e.target.value)} />
      </label>
      <label className="block text-[11px] font-bold uppercase tracking-wider text-slate-500">
        {WORDS.edit.body}
        <textarea className={`${inputCls} mt-1 min-h-40 normal-case font-normal`} value={body} data-no-i18n
          onChange={(e) => setBody(e.target.value)} />
      </label>
      <div className="flex justify-end gap-2">
        <button type="button" className="px-3 py-1.5 text-xs rounded-lg border border-slate-200 dark:border-white/10" onClick={onClose}>{WORDS.edit.cancel}</button>
        <button type="button" disabled={!changed || !summary.trim()} onClick={() => onSave({ summary, body })}
          className="px-3 py-1.5 text-xs font-semibold rounded-lg bg-blue-600 text-white hover:bg-blue-700 disabled:opacity-50">{WORDS.edit.save}</button>
      </div>
    </ModalShell>
  );
};
