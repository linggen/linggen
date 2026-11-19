/**
 * The rows a batch delete removed: those whose delete the engine answered
 * with success. A refused one stays listed (and the chat stays on it).
 * `results` are `Promise.allSettled` over `ids`, in order. Pure, so node
 * tests can hold it.
 */
export function removedIds(ids: string[], results: PromiseSettledResult<unknown>[]): Set<string> {
  return new Set(ids.filter((_, i) => results[i]?.status === 'fulfilled'));
}
