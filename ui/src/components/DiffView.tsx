import React from 'react';

interface DiffLine {
  type: 'add' | 'del' | 'context' | 'hunk';
  oldNum?: number;
  newNum?: number;
  text: string;
}

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

function parseDiff(diff: string): DiffLine[] {
  const lines = diff.split('\n');
  const result: DiffLine[] = [];
  let oldNum = 0;
  let newNum = 0;

  for (const line of lines) {
    // Skip metadata lines
    if (
      line.startsWith('diff --git') ||
      line.startsWith('index ') ||
      line.startsWith('--- ') ||
      line.startsWith('+++ ') ||
      line.startsWith('\\ No newline')
    ) {
      continue;
    }

    // Hunk header
    const hunkMatch = line.match(/^@@ -(\d+)(?:,\d+)? \+(\d+)(?:,\d+)? @@(.*)/);
    if (hunkMatch) {
      oldNum = parseInt(hunkMatch[1], 10);
      newNum = parseInt(hunkMatch[2], 10);
      result.push({ type: 'hunk', text: line });
      continue;
    }

    if (line.startsWith('+')) {
      result.push({ type: 'add', newNum, text: line.slice(1) });
      newNum++;
    } else if (line.startsWith('-')) {
      result.push({ type: 'del', oldNum, text: line.slice(1) });
      oldNum++;
    } else if (line.startsWith(' ')) {
      result.push({ type: 'context', oldNum, newNum, text: line.slice(1) });
      oldNum++;
      newNum++;
    }
  }

  return result;
}

const lineStyles: Record<string, string> = {
  add: 'bg-green-100/60 dark:bg-green-950/40 text-green-900 dark:text-green-200',
  del: 'bg-red-100/60 dark:bg-red-950/40 text-red-900 dark:text-red-200',
  context: '',
  hunk: 'bg-blue-50 dark:bg-blue-950/30 text-blue-600 dark:text-blue-400',
};

export default function DiffView({ diff }: { diff: string }) {
  const parsed = React.useMemo(() => parseDiff(diff), [diff]);

  if (parsed.length === 0) return null;

  return (
    <div className="max-h-80 overflow-auto custom-scrollbar rounded border border-slate-200 dark:border-white/10 bg-white dark:bg-black/30">
      <pre className="font-mono text-[12px] leading-5 whitespace-pre-wrap m-0">
        {parsed.map((line, i) => {
          if (line.type === 'hunk') {
            return (
              <div key={i} className={`px-2 py-0.5 ${lineStyles.hunk}`}>
                {line.text}
              </div>
            );
          }
          const prefix = line.type === 'add' ? '+' : line.type === 'del' ? '-' : ' ';
          return (
            <div key={i} className={`flex ${lineStyles[line.type]}`}>
              <span className="text-slate-400 dark:text-slate-600 select-none w-10 text-right shrink-0 px-1">
                {line.type === 'add' ? '' : line.oldNum}
              </span>
              <span className="text-slate-400 dark:text-slate-600 select-none w-10 text-right shrink-0 px-1">
                {line.type === 'del' ? '' : line.newNum}
              </span>
              <span className="select-none w-4 text-center shrink-0 opacity-60">{prefix}</span>
              <span className="flex-1 px-1">{line.text}</span>
            </div>
          );
        })}
      </pre>
    </div>
  );
}
