// A notice of Ling's work Yinyue said aloud while the person was away
// (`server/resident/herald.rs`): never a row of the thread — the thing it is
// about carries one folded mark of it instead. A run's reply carries it on
// the run's last row; a waiting question carries it on its card. The engine
// sends it live (`notice_told`) and on load (`told_aloud_at`,
// `told_aloud_line` on the row's meta or the listed question).

export type ToldAloud = { atMs: number; line: string };

/** The mark a row's meta or a listed question carries, if any. */
export function toldOf(it: { told_aloud_at?: unknown; told_aloud_line?: unknown } | null | undefined): ToldAloud | undefined {
  const at = it?.told_aloud_at;
  if (typeof at !== 'number') return undefined;
  return { atMs: at * 1000, line: typeof it?.told_aloud_line === 'string' ? it.told_aloud_line : '' };
}

/** The mark from a live `notice_told` event's data. */
export function toldOfEvent(data: { at?: unknown; line?: unknown } | null | undefined): ToldAloud | undefined {
  return toldOf({ told_aloud_at: data?.at, told_aloud_line: data?.line });
}

/** `messages` with run `runId`'s last message marked; unchanged when the
 *  page holds none of that run. */
export function markRun<M extends { runId?: string; toldAloud?: ToldAloud }>(messages: M[], runId: string, told: ToldAloud): M[] {
  for (let i = messages.length - 1; i >= 0; i--) {
    if (messages[i].runId !== runId) continue;
    const next = [...messages];
    next[i] = { ...messages[i], toldAloud: told };
    return next;
  }
  return messages;
}

/** The mark's one folded line. */
export function toldLabel(told: ToldAloud): string {
  const at = new Date(told.atMs);
  const hh = String(at.getHours()).padStart(2, '0');
  const mm = String(at.getMinutes()).padStart(2, '0');
  return `Yinyue said this while you were away · ${hh}:${mm}`;
}
