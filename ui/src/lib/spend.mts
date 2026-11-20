/**
 * What a session's top bar says about spend — the one place that decides it,
 * for every runtime (SessionSpend.tsx only draws it):
 *  - a Claude SDK session: the SDK's own cost figure;
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

/** The engine's answer (`AgentManager::session_spend`): the same one `agent_status` gives. */
export interface SessionSpend {
  models: ModelSpend[];
  tokens: { input: number; output: number; cache_read: number; cache_write: number };
  cost_usd?: number;
  unpriced?: string[];
}

export interface SpendFacts {
  /** The session runs on the Claude SDK; `claudeCost` is its own figure, absent until it reports one. */
  claude: boolean;
  claudeCost?: number;
  /** The chosen model signs in with a ChatGPT account. */
  onChatGpt: boolean;
  usage?: SessionSpend | null;
}

export type SpendView = {
  /** The Claude SDK's figure: `null` while it has none yet. */
  claude?: { cost: number | null };
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

/** The prompt (everything sent) and what was read from the cache. */
function promptOf(t: SessionSpend['tokens']): { sent: number; cached: number } {
  return { sent: t.input + t.cache_read + t.cache_write, cached: t.cache_read };
}

function breakdown(t: SessionSpend['tokens']): string {
  const { sent, cached } = promptOf(t);
  return `${compactTokens(sent)} in, ${compactTokens(t.output)} out, ${compactTokens(cached)} cached`;
}

/** The tokens of the ledger rows that were priced (`priced`), or the metered rows that were not. */
function tokensOfModels(u: SessionSpend, priced: boolean): SessionSpend['tokens'] {
  const sum = { input: 0, output: 0, cache_read: 0, cache_write: 0 };
  for (const m of u.models ?? []) {
    const has = m.cost_usd !== undefined;
    const skip = priced ? !has : has || m.billing !== 'metered';
    if (skip) continue;
    sum.input += m.input; sum.output += m.output; sum.cache_read += m.cache_read; sum.cache_write += m.cache_write;
  }
  return sum;
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
    title: `Estimate at list price for the priced models: ${breakdown(tokensOfModels(u, true))}. Your bill may differ. `
      + `Not priced, so not in the amount: ${unpriced.join(', ')} (${breakdown(tokensOfModels(u, false))}).`,
  };
}

function tokensOf(u: SessionSpend): NonNullable<SpendView['tokens']> {
  const { sent } = promptOf(u.tokens);
  return {
    text: `${compactTokens(sent)} in · ${compactTokens(u.tokens.output)} out`,
    title: `Tokens used: ${breakdown(u.tokens)}. No price is known for this model, so no cost is shown.`,
  };
}

/** Whether any model paid by the token has used anything. */
function spends(u?: SessionSpend | null): u is SessionSpend {
  if (!u) return false;
  const t = u.tokens;
  return t.input + t.output + t.cache_read + t.cache_write > 0;
}

export function spendView(f: SpendFacts): SpendView {
  if (f.claude) return { claude: { cost: f.claudeCost ?? null } };
  const view: SpendView = {};
  if (spends(f.usage)) {
    if (f.usage.cost_usd !== undefined) view.estimate = estimateOf(f.usage, f.usage.cost_usd);
    else view.tokens = tokensOf(f.usage);
  }
  if (f.onChatGpt) view.allowance = true;
  return view;
}
