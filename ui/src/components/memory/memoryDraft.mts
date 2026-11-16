/** The editable copy of a row, and the patch an edit makes. Pure. */

import type { MemoryRow, Patch, Tier } from './memoryApi.mts';

/** Text fields hold '' for "none", so an input is always controlled. */
export interface Draft {
  content: string;
  summary: string;
  indexed: boolean;
  type: string;
  from: string;
  tier: Tier;
  outcome: string;
  scope: string;
}

export function draftOf(row: MemoryRow): Draft {
  return {
    content: row.content ?? '',
    summary: row.summary ?? '',
    indexed: row.indexed === true,
    type: row.type ?? 'fact',
    from: row.from ?? 'derived',
    tier: row.tier ?? 'semantic',
    outcome: row.outcome ?? '',
    scope: row.scope ?? '',
  };
}

/** A nullable field's change: set to the new text, or cleared. */
function nullable(before: string, after: string): string | null | undefined {
  if (before === after) return undefined;
  return after === '' ? null : after;
}

/** What differs between the saved row and its edited copy; `null` = nothing. */
export function patchOf(row: MemoryRow, draft: Draft): Patch | null {
  const was = draftOf(row);
  const patch: Patch = {};
  if (draft.content !== was.content) patch.content = draft.content;
  if (draft.indexed !== was.indexed) patch.indexed = draft.indexed;
  if (draft.type !== was.type) patch.type = draft.type;
  if (draft.from !== was.from) patch.from = draft.from;
  if (draft.tier !== was.tier) patch.tier = draft.tier;
  const summary = nullable(was.summary, draft.summary);
  if (summary !== undefined) patch.summary = summary;
  const outcome = nullable(was.outcome, draft.outcome);
  if (outcome !== undefined) patch.outcome = outcome;
  const scope = nullable(was.scope, draft.scope.trim());
  if (scope !== undefined) patch.scope = scope;
  return Object.keys(patch).length ? patch : null;
}

/** Why a draft cannot be saved, or null. */
export function problemWith(draft: Draft): string | null {
  return draft.content.trim() ? null : 'Content is required.';
}

/** Fields the editor shows itself; the rest go under "All fields". */
export const SHOWN_FIELDS = new Set([
  'id', 'content', 'summary', 'indexed', 'type', 'from', 'tier', 'outcome', 'scope',
  'created_at', 'updated_at', 'occurred_at', 'host', 'source_session',
]);
