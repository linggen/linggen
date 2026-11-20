/**
 * What a session's top bar says about spend — the one place that decides it,
 * (SessionSpend.tsx only draws it):
 *  - tokens a pay-per-token model used: an estimate in dollars when the
 *    engine knew a price, else the tokens and no dollars — never a guess;
 *  - a ChatGPT-account model: its allowance (the meter draws itself).
 * Account and local models have no per-token price and add nothing.
 */

export type Billing = 'metered' | 'account' | 'local' | 'unknown';

export interface ModelSpend {
  model: string;
  billing: Billing;
  calls: number;
  unreported: number;
  input: number;
  output: number;
  cache_read: number;
  cache_write: number;
  cost_usd?: number;
  source?: 'built-in' | 'config';
  as_of?: string;
}

/** A call's counts as the engine sends them; `prompt` is everything sent (`Tokens::prompt`). */
export interface Tokens { input: number; output: number; cache_read: number; cache_write: number; prompt: number }

/**
 * The engine's answer (`server::session_cost`, its `usage`): the same one
 * `agent_status` gives. Every count and rule is the engine's
 * (`pricing.rs` `spend_of`): which tokens were priced, which not, and
 * whether the session spent at all — this file only words them.
 */
export interface SessionSpend {
  models: ModelSpend[];
  tokens: Tokens;
  priced_tokens: Tokens;
  unpriced_tokens: Tokens;
  spent: boolean;
  cost_usd?: number;
  unpriced?: string[];
}

export interface SpendFacts {
  /** The chosen model signs in with a ChatGPT account. */
  onChatGpt: boolean;
  usage?: SessionSpend | null;
}

export type SpendView = {
  /** A priced model's estimate, with the tokens behind it. */
  estimate?: { text: string; title: string };
  /** An unpriced pay-per-token model: tokens, no dollars. */
  tokens?: { text: string; title: string };
  /** The ChatGPT allowance meter belongs here. */
  allowance?: true;
};

/** 38,100,000 → "38.1M"; 12,400 → "12.4k"; 900 → "900". */
export function compactTokens(n: number): string {
  if (n >= 1_000_000) return `${(n / 1_000_000).toFixed(1)}M`;
  if (n >= 1000) return `${(n / 1000).toFixed(1)}k`;
  return String(n);
}

/** Dollars to the cent, or to a tenth of a cent under ten cents. */
export function dollars(cost: number): string {
  return cost >= 0.1 ? cost.toFixed(2) : cost.toFixed(3);
}

function breakdown(t: Tokens): string {
  return `${compactTokens(t.prompt)} in, ${compactTokens(t.output)} out, ${compactTokens(t.cache_read)} cached`;
}

function estimateOf(u: SessionSpend, cost: number): NonNullable<SpendView['estimate']> {
  const unpriced = u.unpriced ?? [];
  if (!unpriced.length) {
    return {
      text: `≈ $${dollars(cost)}`,
      title: `Estimate at list price from this session's tokens: ${breakdown(u.tokens)}. Your bill may differ.`,
    };
  }
  // Some models have no known price: the amount carries a mark, and the
  // hover tells their tokens from the priced ones.
  return {
    text: `≈ $${dollars(cost)}+`,
    title: `Estimate at list price for the priced models: ${breakdown(u.priced_tokens)}. Your bill may differ. `
      + `Not priced, so not in the amount: ${unpriced.join(', ')} (${breakdown(u.unpriced_tokens)}).`,
  };
}

function tokensOf(u: SessionSpend): NonNullable<SpendView['tokens']> {
  return {
    text: `${compactTokens(u.tokens.prompt)} in · ${compactTokens(u.tokens.output)} out`,
    title: `Tokens used: ${breakdown(u.tokens)}. No price is known for this model, so no cost is shown.`,
  };
}

/** Whether any model paid by the token has used anything: the engine's `spent`. */
function spends(u?: SessionSpend | null): u is SessionSpend {
  return !!u?.spent;
}

export function spendView(f: SpendFacts): SpendView {
  const view: SpendView = {};
  if (spends(f.usage)) {
    if (f.usage.cost_usd !== undefined) view.estimate = estimateOf(f.usage, f.usage.cost_usd);
    else view.tokens = tokensOf(f.usage);
  }
  if (f.onChatGpt) view.allowance = true;
  return view;
}
