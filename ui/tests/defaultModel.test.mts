// node --test ui/tests/*.test.mts
// The picker's "Default" names what the engine says the session runs on, and
// the page never works it out from the routing list.
import { test } from 'node:test';
import assert from 'node:assert/strict';
import { readFileSync } from 'node:fs';
import { defaultOptionText, modelInUse } from '../src/lib/defaultModel.mts';

const nameOf = (id: string) => ({ 'deepseek-flash': 'DeepSeek Flash', 'gpt-6.1-terra': 'GPT-6.1 Terra' } as Record<string, string>)[id] ?? id;

test('the option names the engine\'s model, by its readable name', () => {
  assert.equal(defaultOptionText('deepseek-flash', nameOf), 'Default (DeepSeek Flash)');
  assert.equal(defaultOptionText('gpt-6.1-terra', nameOf), 'Default (GPT-6.1 Terra)');
});

test('with no answer yet the option says Default alone, never a guess', () => {
  assert.equal(defaultOptionText(null, nameOf), 'Default');
  assert.equal(defaultOptionText(undefined, nameOf), 'Default');
});

test('the model in use is the picked one, else the engine\'s', () => {
  assert.equal(modelInUse('gpt-6.1-terra', 'deepseek-flash'), 'gpt-6.1-terra');
  assert.equal(modelInUse(null, 'deepseek-flash'), 'deepseek-flash');
  assert.equal(modelInUse(null, null), null);
});

test('the selectors never read the routing list for what "Default" is', () => {
  const text = readFileSync(new URL('../src/components/chat/SessionSelectors.tsx', import.meta.url), 'utf8');
  assert.ok(!/defaultModels\[0\]/.test(text), 'the routing list\'s first entry is not the session\'s default');
});
