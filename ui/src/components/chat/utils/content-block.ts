import type { ContentBlock } from '../../../types';

/** Smart-truncate a string that looks like a file path: keep filename, collapse front. */
export const truncatePath = (p: string, maxLen: number): string => {
  if (p.length <= maxLen) return p;
  const lastSlash = p.lastIndexOf('/');
  if (lastSlash < 0) return p.slice(0, maxLen - 1) + '…';
  const filename = p.slice(lastSlash);
  const secondLast = p.lastIndexOf('/', lastSlash - 1);
  const tail = secondLast >= 0 ? p.slice(secondLast) : filename;
  if (tail.length + 1 <= maxLen) return '…' + tail;
  if (filename.length + 1 <= maxLen) return '…' + filename;
  return filename.slice(0, maxLen - 1) + '…';
};

/** Truncate a display detail: path-aware for file paths, end-truncate otherwise. */
export const truncateDetail = (detail: string, maxLen: number): string => {
  if (detail.length <= maxLen) return detail;
  if (detail.includes('/')) return truncatePath(detail, maxLen);
  return detail.slice(0, maxLen - 1) + '…';
};

/** Extract a display summary from ContentBlock args JSON for compact rendering. */
export const contentBlockSummary = (block: ContentBlock): string => {
  const tool = block.tool || '';
  const raw = block.args || '';
  try {
    const args = JSON.parse(raw);
    switch (tool) {
      case 'Read': return args.file_path || args.path || raw;
      case 'Write': return args.file_path || args.path || raw;
      case 'Edit': return args.file_path || args.path || raw;
      case 'Bash': {
        const cmd = args.command || args.cmd || '';
        return cmd.length > 70 ? cmd.slice(0, 67) + '…' : cmd;
      }
      case 'Grep': return args.pattern || raw;
      case 'Glob': return args.pattern || raw;
      case 'Task':
      case 'delegate_to_agent':
        return args.agent_id || args.agent || raw;
      case 'WebFetch': return args.url || raw;
      case 'WebSearch': return args.query || raw;
      case 'Skill': return args.skill || raw;
      default: {
        const first = Object.values(args).find(v => typeof v === 'string' && (v as string).length < 80) as string | undefined;
        return first || raw;
      }
    }
  } catch {
    return raw.length > 70 ? raw.slice(0, 67) + '…' : raw;
  }
};

/** Build a simple unified diff from old/new strings with optional start line. */
export const buildInlineDiff = (oldStr: string, newStr: string, startLine?: number): string => {
  const oldLines = oldStr.split('\n');
  const newLines = newStr.split('\n');
  const start = startLine || 1;
  const lines: string[] = [];
  lines.push(`@@ -${start},${oldLines.length} +${start},${newLines.length} @@`);
  for (const l of oldLines) lines.push(`-${l}`);
  for (const l of newLines) lines.push(`+${l}`);
  return lines.join('\n');
};
