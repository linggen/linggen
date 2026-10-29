// What deleting a session would stop (GET /api/sessions/running): the reply
// being written, its background tasks and its subagents. The confirm before
// a delete names them; nothing running, nothing to name.

export interface RunningWork {
  turn: boolean;
  tasks: number;
  subagents: number;
}

export const NO_WORK: RunningWork = { turn: false, tasks: 0, subagents: 0 };

/** Several sessions' work, as one. */
export function addWork(all: RunningWork[]): RunningWork {
  return all.reduce((sum, w) => ({
    turn: sum.turn || !!w.turn,
    tasks: sum.tasks + (w.tasks || 0),
    subagents: sum.subagents + (w.subagents || 0),
  }), NO_WORK);
}

const count = (n: number, one: string, many: string) => (n === 1 ? one : many.replace('{n}', String(n)));

/** "2 background tasks and 1 subagent are still running. Deleting stops
 *  them." — or null when nothing runs. `say` puts each piece of English into
 *  the page's language (lib/i18n.ts `translate`); the slots carry over. */
export function runningWorkMessage(work: RunningWork, say: (en: string) => string = (s) => s): string | null {
  const parts: string[] = [];
  if (work.turn) parts.push(say('The current reply'));
  if (work.tasks > 0) parts.push(say(count(work.tasks, '1 background task', '{n} background tasks')));
  if (work.subagents > 0) parts.push(say(count(work.subagents, '1 subagent', '{n} subagents')));
  const [a, b, c] = parts;
  const single = parts.length === 1 && (work.turn || work.tasks === 1 || work.subagents === 1);
  switch (parts.length) {
    case 0: return null;
    case 1: return say(single
      ? `${a} is still running. Deleting stops it.`
      : `${a} are still running. Deleting stops them.`);
    case 2: return say(`${a} and ${b} are still running. Deleting stops them.`);
    default: return say(`${a}, ${b} and ${c} are still running. Deleting stops them.`);
  }
}
