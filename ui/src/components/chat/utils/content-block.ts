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
      case 'Memory_query': {
        // verb=search → show the query; verb=get → show the id;
        // verb=list → show the filter or "list" itself.
        const verb = args.verb || '';
        if (verb === 'search' && args.query) return `"${args.query}"`;
        if (verb === 'get' && args.id) return args.id;
        if (verb === 'list') return args.type || args.tier || 'list';
        return args.query || args.id || verb || raw;
      }
      case 'Memory_write': {
        // verb=add/update → show the content snippet; verb=delete → show the id.
        const verb = args.verb || '';
        if (verb === 'delete' && args.id) return `delete ${args.id}`;
        if (args.content) return `"${args.content}"`;
        if (args.id) return args.id;
        return verb || raw;
      }
      default: {
        const first = Object.values(args).find(v => typeof v === 'string' && (v as string).length < 80) as string | undefined;
        return first || raw;
      }
    }
  } catch {
    return raw.length > 70 ? raw.slice(0, 67) + '…' : raw;
  }
};

/** Build a unified diff from old/new strings with context lines (common prefix/suffix). */
export const buildInlineDiff = (oldStr: string, newStr: string, startLine?: number): string => {
  const oldLines = oldStr.split('\n');
  const newLines = newStr.split('\n');
  const start = startLine || 1;

  // Find common prefix lines
  let prefixLen = 0;
  const maxPrefix = Math.min(oldLines.length, newLines.length);
  while (prefixLen < maxPrefix && oldLines[prefixLen] === newLines[prefixLen]) {
    prefixLen++;
  }

  // Find common suffix lines (don't overlap with prefix)
  let suffixLen = 0;
  const maxSuffix = Math.min(oldLines.length - prefixLen, newLines.length - prefixLen);
  while (
    suffixLen < maxSuffix &&
    oldLines[oldLines.length - 1 - suffixLen] === newLines[newLines.length - 1 - suffixLen]
  ) {
    suffixLen++;
  }

  // Limit context to 3 lines (like standard unified diff)
  const ctxBefore = Math.min(prefixLen, 3);
  const ctxAfter = Math.min(suffixLen, 3);

  const changedOld = oldLines.slice(prefixLen, oldLines.length - suffixLen);
  const changedNew = newLines.slice(prefixLen, newLines.length - suffixLen);

  const hunkOldStart = start + prefixLen - ctxBefore;
  const hunkOldLen = ctxBefore + changedOld.length + ctxAfter;
  const hunkNewStart = start + prefixLen - ctxBefore;
  const hunkNewLen = ctxBefore + changedNew.length + ctxAfter;

  const lines: string[] = [];
  lines.push(`@@ -${hunkOldStart},${hunkOldLen} +${hunkNewStart},${hunkNewLen} @@`);

  // Context before
  for (let i = prefixLen - ctxBefore; i < prefixLen; i++) {
    lines.push(` ${oldLines[i]}`);
  }
  // Removed lines
  for (const l of changedOld) lines.push(`-${l}`);
  // Added lines
  for (const l of changedNew) lines.push(`+${l}`);
  // Context after
  for (let i = oldLines.length - suffixLen; i < oldLines.length - suffixLen + ctxAfter; i++) {
    lines.push(` ${oldLines[i]}`);
  }

  return lines.join('\n');
};
