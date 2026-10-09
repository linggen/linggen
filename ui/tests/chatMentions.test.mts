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

import { completeFileMention, mentionInProgress, isBareMention, floorOf, prefillAfter, PREFILL_QUIET_MS } from '../src/lib/chatMentions.mts';

test('@ opens agents only at the start; /f opens files', () => {
  assert.deepEqual(mentionInProgress('@y'), { kind: 'lead-agent', filter: 'y' });
  assert.equal(mentionInProgress('hi @y'), null);
  assert.equal(mentionInProgress('@Yinyue '), null);
  assert.deepEqual(mentionInProgress('/f'), { kind: 'file-search', query: '' });
  assert.deepEqual(mentionInProgress('see /f ab'), { kind: 'file-search', query: 'ab' });
  assert.deepEqual(mentionInProgress('/f src/x'), { kind: 'file-browse', dir: 'src/', filter: 'x' });
  assert.equal(mentionInProgress('/fix'), null);
});

test('completing a file writes @path, a directory stays in /f', () => {
  assert.equal(completeFileMention('look /f ab', 'a/b.ts', true), 'look @a/b.ts ');
  assert.equal(completeFileMention('/f a', 'src/', false), '/f src/');
});

test('pre-fill: her mention while she holds the floor, cleared after 10 minutes', () => {
  const table = ['ling', 'yinyue'];
  const toHer = [{ role: 'user' as const, from: 'user', to: 'yinyue', timestampMs: 1000 }];
  const floor = floorOf(toHer, table);
  assert.deepEqual(floor, { speaker: 'yinyue', at: 1000 });
  assert.equal(prefillAfter(floor, 1000 + 60_000, 'Yinyue'), '@Yinyue ');
  assert.equal(prefillAfter(floor, 1000 + PREFILL_QUIET_MS - 1, 'Yinyue'), '@Yinyue ');
  assert.equal(prefillAfter(floor, 1000 + PREFILL_QUIET_MS, 'Yinyue'), '');
  assert.equal(prefillAfter({}, 1000, 'Yinyue'), '');
});

test('pre-fill: a line to Ling hands the floor back; other rows are not at the table', () => {
  const table = ['ling', 'yinyue'];
  const rows = [
    { role: 'user' as const, to: 'yinyue', timestampMs: 1 },
    { role: 'agent' as const, from: 'yinyue', timestampMs: 2 },
    { role: 'user' as const, to: 'ling', timestampMs: 3 },
    { role: 'agent' as const, from: 'coder-sub', timestampMs: 4 },
    { role: 'user' as const, to: 'system', timestampMs: 5 },
  ];
  assert.deepEqual(floorOf(rows, table), { speaker: 'ling', at: 3 });
  assert.deepEqual(floorOf(rows.slice(0, 2), table), { speaker: 'yinyue', at: 2 });
  assert.equal(prefillAfter(floorOf(rows, table), 4, 'Yinyue'), '');
});

test('a bare mention is an address with nothing said', () => {
  assert.equal(isBareMention('@Yinyue ', AGENTS), true);
  assert.equal(isBareMention('@@银月', AGENTS), true);
  assert.equal(isBareMention('@Yinyue hi', AGENTS), false);
  assert.equal(isBareMention('@src/main.rs', AGENTS), false);
});
