// node --test ui/tests/*.test.mts
// A spent allowance's chat line: which one it was, and what is no such line.
import { test } from 'node:test';
import assert from 'node:assert/strict';
import { billingBody, billingReason } from '../src/lib/billing.mts';

test('the trial and the monthly pool each read from the engine line', () => {
  const trial = billingBody('Error: BILLING_REQUIRED: trial_exhausted: Your free tokens are used up for now.');
  const pool = billingBody('Error: BILLING_REQUIRED: monthly_cap_exceeded: Monthly token limit reached');
  assert.equal(billingReason(trial!), 'trial');
  assert.equal(billingReason(pool!), 'monthly');
});

test('an older prose-only row reads as the trial; other errors are no billing line', () => {
  assert.equal(billingReason(billingBody('Error: BILLING_REQUIRED: Your free tokens are used up')!), 'trial');
  assert.equal(billingBody('Error: The model has hit its limit for now'), null);
  assert.equal(billingBody(undefined), null);
});
