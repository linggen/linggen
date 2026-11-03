// A turn refused on a spent Linggen Cloud allowance reads, in the chat, as
// `Error: BILLING_REQUIRED: <code>: <proxy message>` (provider/openai/mod.rs
// billing_line). An older row carries prose only and reads as the trial.

/** The refusal's body (after `Error:`), or null when `text` is no such line. */
export function billingBody(text?: string): string | null {
  if (!text) return null;
  const body = text.trim().replace(/^Error:\s*/i, '');
  return body.startsWith('BILLING_REQUIRED:') ? body : null;
}

/** Which allowance it spent: the free trial, or a subscriber's monthly pool. */
export function billingReason(body: string): 'trial' | 'monthly' {
  return /monthly_cap_exceeded/.test(body) ? 'monthly' : 'trial';
}
