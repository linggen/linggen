async function trigger(path: string, body: object, conflict: string, failure: string, missing?: string): Promise<string | null> {
  const res = await fetch(path, {
    method: 'POST', headers: { 'Content-Type': 'application/json' },
    body: JSON.stringify(body),
  });
  if (res.status === 409) throw new Error(conflict);
  if (res.status === 404 && missing) throw new Error(missing);
  if (!res.ok) throw new Error(failure);
  const reply = await res.json();
  return typeof reply.session_id === 'string' ? reply.session_id : null;
}

/** Start the dream mission for all days or one attended day. */
export function startDream(day?: string): Promise<string | null> {
  return trigger('/api/missions/dream/trigger', day ? { day, attended: true } : {},
    'A dream is already running.', 'Could not start dream.');
}

export function startScan(day: string): Promise<string | null> {
  return trigger('/api/missions/memory-scan/trigger', { day, attended: true },
    'A scan is already running.', 'Could not start the scan.', 'This needs a newer Linggen.');
}

export function startSolve(): Promise<string | null> {
  return trigger('/api/missions/memory-solve/trigger', { attended: true },
    'Solve is already running.', 'Could not start solve.', 'This needs a newer Linggen.');
}
