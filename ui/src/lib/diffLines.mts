/**
 * A unified diff as the lines it shows, numbered. The `@@` header says how
 * many old and new lines its hunk holds; until they are all read, every line
 * is the hunk's — an added line reading `++ x` arrives as `+++ x`, and is
 * an added line, not a file header. Only between hunks are `diff --git`,
 * `index`, `---` and `+++` headers. Pure, so node tests can hold it; the
 * diff card and its +/- counts both read it.
 */
export interface DiffLine {
  type: 'add' | 'del' | 'context' | 'hunk';
  oldNum?: number;
  newNum?: number;
  text: string;
}

const HUNK = /^@@ -(\d+)(?:,(\d+))? \+(\d+)(?:,(\d+))? @@(.*)/;

export function parseDiff(diff: string): DiffLine[] {
  const result: DiffLine[] = [];
  let oldNum = 0;
  let newNum = 0;
  let oldLeft = 0;
  let newLeft = 0;

  for (const line of diff.split('\n')) {
    if (line.startsWith('\\ No newline')) continue;
    const inHunk = oldLeft > 0 || newLeft > 0;
    if (!inHunk) {
      const hunk = line.match(HUNK);
      if (hunk) {
        oldNum = parseInt(hunk[1], 10);
        oldLeft = hunk[2] === undefined ? 1 : parseInt(hunk[2], 10);
        newNum = parseInt(hunk[3], 10);
        newLeft = hunk[4] === undefined ? 1 : parseInt(hunk[4], 10);
        result.push({ type: 'hunk', text: line });
      }
      // Anything else between hunks is a header.
      continue;
    }
    if (line.startsWith('+')) {
      result.push({ type: 'add', newNum, text: line.slice(1) });
      newNum++;
      newLeft--;
    } else if (line.startsWith('-')) {
      result.push({ type: 'del', oldNum, text: line.slice(1) });
      oldNum++;
      oldLeft--;
    } else {
      // A context line (an empty one may have lost its leading space).
      result.push({ type: 'context', oldNum, newNum, text: line.slice(1) });
      oldNum++;
      newNum++;
      oldLeft--;
      newLeft--;
    }
  }
  return result;
}
