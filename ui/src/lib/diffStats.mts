import { parseDiff } from './diffLines.mts';

/** Count added/deleted lines from a unified diff string — the lines the
 *  diff card shows (lib/diffLines.mts). */
export function diffStats(diff: string): { added: number; deleted: number } {
  let added = 0;
  let deleted = 0;
  for (const line of parseDiff(diff)) {
    if (line.type === 'add') added++;
    else if (line.type === 'del') deleted++;
  }
  return { added, deleted };
}
