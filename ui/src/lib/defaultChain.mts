/**
 * The default model chain after a star or Clear on `modelId`: the default
 * (the chain's first) is taken off the chain and the fallbacks after it
 * stay; any other model becomes the one default. Pure, so node tests can
 * hold it.
 */
export function toggledDefault(chain: string[], modelId: string): string[] {
  return chain[0] === modelId ? chain.slice(1) : [modelId];
}
