// node --test ui/tests/*.test.mts (Node strips the types).
import { test } from 'node:test';
import assert from 'node:assert/strict';
import { leadingAgentMention } from '../src/lib/chatMentions.mts';

const AGENTS = [
  { name: 'ling' },
  { name: 'yinyue', aliases: ['银月', 'Yinyue'] },
  { name: 'lingo' },
];

test('a spaced mention names the agent', () => {
  assert.deepEqual(leadingAgentMention('@银月 你好', AGENTS), { agent: 'yinyue', sticky: false, body: '你好' });
});

test('a CJK name needs no space before the words', () => {
  assert.equal(leadingAgentMention('@银月你好', AGENTS)?.agent, 'yinyue');
  assert.equal(leadingAgentMention('@银月你好', AGENTS)?.body, '你好');
  assert.equal(leadingAgentMention('@@银月，看这一局', AGENTS)?.sticky, true);
});

test('a Latin name followed by CJK words is still the name', () => {
  assert.equal(leadingAgentMention('@yinyue你好', AGENTS)?.agent, 'yinyue');
  assert.equal(leadingAgentMention('@Yinyue: hi', AGENTS)?.body, 'hi');
});

test('the longest name the text starts with wins', () => {
  assert.equal(leadingAgentMention('@lingo hi', AGENTS)?.agent, 'lingo');
  assert.equal(leadingAgentMention('@ling hi', AGENTS)?.agent, 'ling');
});

test('a Latin name never swallows the start of a longer word', () => {
  assert.equal(leadingAgentMention('@lingering thoughts', AGENTS), undefined);
});

test('no words after the name, a path, or a mid-text @ is no mention', () => {
  assert.equal(leadingAgentMention('@银月', AGENTS), undefined);
  assert.equal(leadingAgentMention('@src/main.rs fix it', AGENTS), undefined);
  assert.equal(leadingAgentMention('hello @银月', AGENTS), undefined);
});
