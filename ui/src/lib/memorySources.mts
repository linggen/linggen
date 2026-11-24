/**
 * Settings → Notes: what Linggen remembers, by source (`GET /api/memory-sources`,
 * `server/api/memory_sources.rs`) — Linggen's own notes, which the owner may
 * tend, and what other agents keep, which is shown and never changed. The
 * engine decides what holds; the page shows its answer and sends the owner's
 * change.
 *
 * Every sentence the section shows is in `WORDS`, at most twenty words each
 * (tests/memorySources.test.mts holds that, and a Chinese entry for each).
 */

/** The one source whose notes can be changed. */
export const OWN_SOURCE = 'linggen';
/** The tab of Linggen's notes to look at again. */
export const REVIEW_TAB = 'review';

export interface Source {
  id: string;
  label: string;
  own: boolean;
  count: number;
  topics: number;
  voided?: number;
  review?: number;
  unreadable?: string[];
}

export interface Topic {
  path: string;
  name: string;
  count: number;
  voided?: number;
  scope?: string;
  /** An own topic: the newest `updated` day. Another agent's: epoch seconds. */
  changed: string | number;
}

export interface Note {
  id: string;
  summary: string;
  body: string;
  kind: string;
  voice: string;
  created: string;
  updated: string;
  source: string;
  void: string | null;
  no_raise: boolean;
  topic: string;
  scope: string;
  path: string;
  version: string;
}

export interface ReviewNote extends Note {
  why: 'void' | 'unused';
}

export interface FileEntry {
  path: string;
  name: string;
  bytes: number;
  changed: number;
}

/** What the owner can do to one of Linggen's own notes, in menu order (the destructive one last). */
export type NoteAction = 'edit' | 'void' | 'unvoid' | 'no-raise' | 'raise' | 'delete';

export function noteActions(note: Pick<Note, 'void' | 'no_raise'>): NoteAction[] {
  return [
    'edit',
    note.void === null ? 'void' : 'unvoid',
    note.no_raise ? 'raise' : 'no-raise',
    'delete',
  ];
}

export function actionLabel(action: NoteAction): string {
  return WORDS.actions[action];
}

/** The page's own words for what the engine refused with (`Refusal::code`). */
export function refusalLine(code: string | undefined): string {
  return (code && WORDS.refused[code as keyof typeof WORDS.refused]) || WORDS.refusedOther;
}

/** The engine's `{ "error": code }` out of an ApiError's message, if it is one. */
export function refusalCode(message: string): string | undefined {
  try {
    const body = JSON.parse(message);
    return typeof body?.error === 'string' ? body.error : undefined;
  } catch {
    return undefined;
  }
}

export function countLine(n: number): string {
  return n === 1 ? '1 note' : `${n} notes`;
}

export function fileCountLine(n: number): string {
  return n === 1 ? '1 file' : `${n} files`;
}

export function sizeLine(bytes: number): string {
  return bytes < 1024 ? `${bytes} B` : `${(bytes / 1024).toFixed(bytes < 10240 ? 1 : 0)} KB`;
}

/** `2026-11-24` for epoch seconds, in the viewer's zone. */
export function dayOf(secs: number): string {
  const d = new Date(secs * 1000);
  return [d.getFullYear(), String(d.getMonth() + 1).padStart(2, '0'), String(d.getDate()).padStart(2, '0')].join('-');
}

/** The one line under a note: kind, who wrote it, when. */
export function noteMeta(note: Pick<Note, 'kind' | 'source' | 'updated'>): string {
  return `${note.kind} · ${note.source} · ${note.updated}`;
}

/** Why a note is in the review list, in a line. */
export function reviewLine(note: Pick<ReviewNote, 'why' | 'void'>): string {
  return note.why === 'void' ? `${WORDS.markedWrong}: ${note.void ?? ''}` : WORDS.unused;
}

export const WORDS = {
  title: 'What Linggen remembers',
  intro: 'Linggen’s own notes you can fix. What other agents keep is shown, never changed.',
  loading: 'Loading…',
  nothing: 'Nothing here yet.',
  noTopics: 'No topics yet.',
  noEntries: 'Nothing in this topic.',
  review: 'To review',
  reviewHint: 'Notes marked wrong, or not read for 90 days.',
  reviewEmpty: 'Nothing to review.',
  markedWrong: 'Marked wrong',
  unused: 'Not read for 90 days',
  topics: 'Topics',
  readOnly: 'Read only. This belongs to another agent; open the file there to change it.',
  fileAt: 'File',
  truncated: 'Shown in part.',
  voidTag: 'Wrong',
  noRaiseTag: 'Not raised',
  userTag: 'In your words',
  options: 'Options',
  unreadable: 'Some files could not be read',
  edit: {
    title: 'Edit note',
    summary: 'Summary',
    body: 'Details',
    save: 'Save',
    cancel: 'Cancel',
  },
  askReason: 'Why is this note wrong?',
  askDelete: 'Delete this note? Its file’s last version stays in history.',
  failed: 'Could not load.',
  actions: {
    edit: 'Edit',
    void: 'Mark wrong',
    unvoid: 'Mark right again',
    'no-raise': 'Never bring up',
    raise: 'Allow bringing up',
    delete: 'Delete',
  },
  refused: {
    status: 'Not a memory: status or progress. Keep a rule, a decision, or a fact.',
    empty: 'This cannot be empty.',
    too_long: 'This is too long.',
    not_found: 'This note is gone. The list has been refreshed.',
    conflict: 'This note changed meanwhile. The list has been refreshed; try again.',
    read_again: 'This topic changed meanwhile. The list has been refreshed; try again.',
    duplicate: 'Another note already says this.',
    disk: 'Could not save to disk.',
  },
  refusedOther: 'Could not change this note.',
} as const;

/** Every sentence the section can show, for the test that counts words. */
export function allWords(): string[] {
  const out: string[] = [];
  const walk = (value: unknown) => {
    if (typeof value === 'string') out.push(value);
    else if (value && typeof value === 'object') Object.values(value).forEach(walk);
  };
  walk(WORDS);
  return out;
}
