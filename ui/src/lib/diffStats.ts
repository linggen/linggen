/** Count added/deleted lines from a unified diff string. */
export function diffStats(diff: string): { added: number; deleted: number } {
  let added = 0;
  let deleted = 0;
  for (const line of diff.split('\n')) {
    if (line.startsWith('+') && !line.startsWith('+++')) added++;
    else if (line.startsWith('-') && !line.startsWith('---')) deleted++;
  }
  return { added, deleted };
}
