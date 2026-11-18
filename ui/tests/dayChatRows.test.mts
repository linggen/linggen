import { test } from 'node:test';
import assert from 'node:assert/strict';
import { selectMainChatRows } from '../src/lib/chatRows.mts';
import { HOME_PLACE, foldOthers, foldLabel, withLivePlaces, streamsHere } from '../src/lib/dayFold.mts';
import type { ChatMessage } from '../src/types.ts';

const row = (from: string, to: string, extra: Partial<ChatMessage>): ChatMessage =>
  ({ role: from === 'user' ? 'user' : 'agent', from, to, text: 'line', timestamp: '', ...extra });

// Two independent page views of the same day's live rows, then both reload.
// Neither page has the other page's speaker in its roster.
test('home and Lingjing count both sides of the other place while live and after reload', () => {
  const turns = [
    { place: HOME_PLACE, agent: 'ling', runId: 'home' },
    { place: 'lingjing', agent: 'yinyue', runId: 'game' },
  ];
  const places = Object.fromEntries(turns.map((t) => [t.runId, t.place]));
  const live = turns.flatMap(({ place, agent, runId }) => [
    row('user', agent, { place }), row(agent, 'user', { runId, isGenerating: true }),
  ]);
  const saved = withLivePlaces(live, places).map((m) => ({ ...m, isGenerating: false }));
  const firstTurn = live.slice(0, 2);
  const secondTurn = [...saved.slice(0, 2), ...live.slice(2)];

  for (const here of turns) {
    const other = turns.find((t) => t.place !== here.place)!;
    const view = (rows: ChatMessage[], runPlaces: Record<string, string>) => {
      const selected = selectMainChatRows(rows, { selected: here.agent, mainAgentIds: [here.agent], here: here.place, runPlaces });
      const placed = withLivePlaces(selected, runPlaces);
      return foldOthers(placed, here.place).flatMap((i) => i.kind === 'fold'
        ? [foldLabel(i.place, i.indexes.length, i.indexes.some((j) => !!placed[j].isGenerating))] : []);
    };
    assert.deepEqual(view(here.place === HOME_PLACE ? secondTurn : firstTurn, places), [`${other.place} · working…`]);
    assert.deepEqual(view(saved, {}), [`${other.place} · 2 messages`]);
    assert.equal(streamsHere({ run_id: other.runId }, places, here.place), false);
    assert.equal(streamsHere({ run_id: here.runId }, places, here.place), true);
  }
});

test('normal and private chat rows have no daily place and keep the existing member filter', () => {
  const own = row('ling', 'user', {});
  const other = row('yinyue', 'user', {});
  assert.deepEqual(selectMainChatRows([own, other], { here: 'private', selected: 'ling' }), [own]);
});
