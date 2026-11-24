// The chat's thinking-effort chip, as data: which values it lists and which is
// selected. The engine says what a model offers (`effort` on each model of the
// list, `ModelManager::effort_choices`); nothing here knows a provider or a
// model name. Words are `W` of lib/modelsWords.mts, put into Chinese by the page.
import { W } from './modelsWords.mts';

export interface ChipModel { id: string; effort?: string[] | null }

/** The fixed word of each value the engine can offer, as the Models page says it. */
export const EFFORT_WORD: Record<string, string> = {
  low: W.effortLow, medium: W.effortMedium, high: W.effortHigh, xhigh: W.effortXhigh, max: W.effortMax,
};

export interface Chip {
  /** What the model offers, in the engine's order. */
  values: string[];
  /** The stored value if the model offers it; '' (automatic) otherwise. */
  selected: string;
}

/** The chip for `modelId`, or null when it offers none (no chip). A stored
 *  value the model does not offer reads as automatic and is not erased. */
export function effortChip(models: readonly ChipModel[], modelId: string | null | undefined, stored: string | null | undefined): Chip | null {
  const values = (models.find((m) => m.id === modelId)?.effort ?? []).filter((v) => v in EFFORT_WORD);
  if (!values.length) return null;
  return { values, selected: stored && values.includes(stored) ? stored : '' };
}
