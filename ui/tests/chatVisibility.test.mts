import { test } from 'node:test';
import assert from 'node:assert/strict';
import { visibleChatRows } from '../src/lib/hiddenRows.mts';
import type { ChatMessage } from '../src/types.ts';

const row = (from: string, text: string, extra: Partial<ChatMessage> = {}): ChatMessage =>
  ({ role: from === 'user' ? 'user' : 'agent', from, text, timestamp: '', ...extra });

const sense = [{ type: 'tool_use' as const, id: 'sense', tool: 'sense' }];

test('Yinyue silence hides the whole run (live or reloaded), not an adjacent turn', () => {
  const work = row('yinyue', '', { runId: 'glance', content: sense, place: 'lingjing' });
  const other = row('yinyue', 'Your fortune', { runId: 'reading', content: sense });
  const user = row('user', 'SILENT', { runId: 'glance' });
  const running = { ...work, isGenerating: true };
  assert.deepEqual(visibleChatRows([running, other, user]), [running, other, user]);
  for (const silent of [
    { ...running, live: true, silent: true },
    row('yinyue', '', { runId: 'glance', silent: true }),
  ]) {
    assert.deepEqual(visibleChatRows([work, other, user, silent]), [other, user]);
  }
  assert.deepEqual(visibleChatRows([row('ling', '', { silent: true, content: sense })]).length, 1);
});

test('internal rows do not interrupt or count toward a fold', () => {
  const visible = row('yinyue', 'Your fortune', { place: 'lingjing' });
  assert.deepEqual(visibleChatRows([
    row('user', '[HIDDEN] read my fortune'), row('world', 'context'), row('system', 'tool observation'), visible,
  ]), [visible]);
});
