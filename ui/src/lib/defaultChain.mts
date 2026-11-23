/**
 * The default model chain after `modelId` is made the default: it goes first,
 * the models that were there stay after it as fallbacks, and it is never in
 * the chain twice. The same rule as `Config::make_default` in the engine
 * (crates/linggen-engine/src/config.rs); the two pages that pick a default
 * (Models, Agents) both say it through here. Pure, so node tests can hold it.
 */
export function madeDefault(chain: string[], modelId: string): string[] {
  return [modelId, ...chain.filter((id) => id !== modelId)];
}
