/** Start the dream mission for all days or one attended day. */
export async function startDream(day?: string): Promise<void> {
  const res = await fetch('/api/missions/dream/trigger', {
    method: 'POST', headers: { 'Content-Type': 'application/json' },
    body: JSON.stringify(day ? { day, attended: true } : {}),
  });
  if (res.status === 409) throw new Error('A dream is already running.');
  if (!res.ok) throw new Error('Could not start dream.');
}
