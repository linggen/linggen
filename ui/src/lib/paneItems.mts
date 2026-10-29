// The subagent pane's one list (doc/background-tasks-spec.md, rule 4): every
// subagent of the session and every background task, each exactly once. A
// background subagent is both a task and a subagent tree entry; the task's
// `subagent_id` is the entry's tracking id, and the two merge into one item.
import type { BackgroundTask, SubagentTreeEntry } from '../types.ts';

export interface PaneItem {
  /** Stable across renders: the subagent's tracking id, else the task id. */
  key: string;
  /** "subagent", "shell", "watch", "workflow" or the runtime's own label. */
  kind: string;
  /** The first line of what it was asked to do. */
  title: string;
  /** A subagent's agent type ("general-purpose"), shown muted beside the title. */
  agentType?: string;
  /** The task's word when there is a task (it decides Stop), else the entry's. */
  status: string;
  running: boolean;
  startedAt?: number;
  endedAt?: number;
  /** Latest activity while it runs, how it ended once it has. */
  line?: string;
  /** A subagent's context window, as the session header shows its own: "ctx 45.0k". */
  context?: string;
  entry?: SubagentTreeEntry;
  task?: BackgroundTask;
}

export function firstLine(text: string | null | undefined): string {
  return String(text ?? '').trim().split('\n')[0].trim();
}

/** A report's first line as a summary: without its markdown heading or bold. */
export function reportLine(report: string | null | undefined): string {
  return firstLine(report).replace(/^#+\s*/, '').replace(/^\*\*(.+)\*\*:?$/, '$1');
}

/** Tokens the way the session header shows its context: "850", "45.0k". */
export function formatTokens(n: number): string {
  return n >= 1000 ? `${(n / 1000).toFixed(1)}k` : String(n);
}

export function formatElapsed(ms: number): string {
  const s = Math.max(0, Math.floor(ms / 1000));
  if (s < 60) return `${s}s`;
  const m = Math.floor(s / 60);
  if (m < 60) return `${m}m ${s % 60}s`;
  return `${Math.floor(m / 60)}h ${m % 60}m`;
}

export const KIND_LABEL: Record<string, string> = { shell: 'Shell', subagent: 'Agent', watch: 'Watch', workflow: 'Workflow' };

export const STATUS_LABEL: Record<string, string> = {
  running: 'Running',
  completed: 'Completed',
  failed: 'Failed',
  stopped: 'Stopped',
  interrupted: 'Interrupted',
};

/** "2m 5s", or nothing for a subagent loaded from history (never stamped)
 *  or one that ended without a task to say when. */
export function elapsedOf(item: PaneItem, now: number): string | null {
  if (!item.startedAt) return null;
  if (item.running) return formatElapsed(now - item.startedAt);
  return item.endedAt ? formatElapsed(item.endedAt - item.startedAt) : null;
}

/** A flat transcript of a subagent's chat: task → tool calls → result.
 *  Plain text so it pastes into any editor / chat window cleanly. */
export function subagentTranscript(entry: SubagentTreeEntry): string {
  const name = entry.agentName || entry.subagentId;
  const lines: string[] = [];
  if (entry.task) {
    lines.push(`From main → ${name}:`);
    lines.push(entry.task);
    lines.push('');
  }
  for (const step of entry.toolSteps || []) {
    const status = step.status === 'done' ? '✓' : step.status === 'failed' ? '✗' : '⏺';
    lines.push(`${status} ${step.toolName}${step.args ? `(${step.args})` : ''}`);
  }
  if (entry.resultText) {
    if (entry.toolSteps && entry.toolSteps.length > 0) lines.push('');
    lines.push(`${name} → main:`);
    lines.push(entry.resultText);
  }
  if (entry.report) {
    lines.push('', 'Report:', entry.report);
  }
  return lines.join('\n');
}

const ENTRY_STATUS: Record<SubagentTreeEntry['status'], string> = { running: 'running', done: 'completed', failed: 'failed' };

function toItem(entry: SubagentTreeEntry | undefined, task: BackgroundTask | undefined): PaneItem {
  const status = task?.status ?? ENTRY_STATUS[entry?.status ?? 'done'];
  const running = status === 'running';
  const ended = reportLine(entry?.report) || task?.summary || firstLine(entry?.resultText);
  return {
    key: entry?.subagentId ?? task!.id,
    kind: entry ? 'subagent' : task!.kind,
    title: firstLine(entry?.task) || firstLine(task?.title) || entry?.agentName || task!.id,
    agentType: entry?.agentName || undefined,
    status,
    running,
    startedAt: task?.started_at ?? entry?.timestampMs,
    endedAt: task?.ended_at,
    line: (running ? entry?.currentActivity || task?.activity : ended) || undefined,
    context: entry?.contextTokens ? `ctx ${formatTokens(entry.contextTokens)}` : undefined,
    entry,
    task,
  };
}

/** Newest first. A subagent's start is stamped only live, so one loaded from
 *  history keeps its place after the stamped ones. */
export function paneItems(entries: readonly SubagentTreeEntry[], tasks: readonly BackgroundTask[]): PaneItem[] {
  const taskBySubagent = new Map(tasks.filter((t) => t.subagent_id).map((t) => [t.subagent_id!, t]));
  const merged = new Set<string>();
  const items = entries.map((entry) => {
    const task = taskBySubagent.get(entry.subagentId);
    if (task) merged.add(task.id);
    return toItem(entry, task);
  });
  for (const task of tasks) {
    if (!merged.has(task.id)) items.push(toItem(undefined, task));
  }
  return items
    .map((item, i) => ({ item, i }))
    .sort((a, b) => (b.item.startedAt ?? 0) - (a.item.startedAt ?? 0) || b.i - a.i)
    .map(({ item }) => item);
}
