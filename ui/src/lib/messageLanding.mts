/** Decide what this surface does with the session returned by a chat send. */
export function messageLanding(source: string | null | undefined, destination: string | null | undefined, embedded: boolean, status?: string): 'stay' | 'adopt' | 'follow' {
  if (!destination || source === destination) return 'stay';
  if (!source) return 'adopt';
  // Refused messages were not kept anywhere; queued messages were carried.
  if (embedded || status === 'absent' || status === 'unavailable') return 'stay';
  return 'follow';
}
