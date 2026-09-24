/** Mission display helpers — pure functions, no I/O. */

const DAY_NAMES: Record<string, string> = {
  '0': 'Sun', '1': 'Mon', '2': 'Tue', '3': 'Wed', '4': 'Thu', '5': 'Fri', '6': 'Sat', '7': 'Sun',
};

export function describeCron(schedule: string): string {
  const parts = schedule.split(/\s+/);
  if (parts.length !== 5) return schedule;
  const [min, hour, dom, mon, dow] = parts;
  if (min === '*' && hour === '*' && dom === '*' && mon === '*' && dow === '*') return 'Every minute';
  if (min.startsWith('*/') && hour === '*' && dom === '*' && mon === '*' && dow === '*') return `Every ${min.slice(2)} min`;
  if (hour.startsWith('*/') && dom === '*' && mon === '*' && dow === '*') return `Every ${hour.slice(2)}h at :${min.padStart(2, '0')}`;
  if (hour === '*' && dom === '*' && mon === '*' && dow === '*') return `Hourly at :${min.padStart(2, '0')}`;
  if (dom === '*' && mon === '*' && dow === '*') return `Daily ${hour}:${min.padStart(2, '0')}`;
  if (dom === '*' && mon === '*' && dow !== '*') {
    if (dow.includes('-')) {
      const [start, end] = dow.split('-');
      return `${DAY_NAMES[start] || start}-${DAY_NAMES[end] || end} ${hour}:${min.padStart(2, '0')}`;
    }
    const days = dow.split(',').map(d => DAY_NAMES[d] || d).join(', ');
    return `${days} ${hour}:${min.padStart(2, '0')}`;
  }
  return schedule;
}

/** Display label for a working folder path — show the last segment. */
export function folderLabel(path: string | null | undefined): string | null {
  if (!path) return null;
  return path.split('/').pop() || path;
}

export const PERMISSION_MODES = [
  { value: 'read', label: 'Read-only', desc: 'Analyze and report only — no Write, Edit, or Bash.', color: 'green' },
  { value: 'edit', label: 'Edit', desc: 'Write + edit files within the working directory and declared paths.', color: 'blue' },
  { value: 'admin', label: 'Admin', desc: 'Full access, no restrictions. Use with caution.', color: 'amber' },
] as const;
