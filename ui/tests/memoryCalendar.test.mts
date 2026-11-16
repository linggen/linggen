import { test } from 'node:test';
import assert from 'node:assert/strict';
import { dayState, monthGrid, monthOf, shiftMonth } from '../src/components/memory/memoryCalendar.mts';

test('month grid starts Monday and pads complete weeks', () => {
  const grid = monthGrid(2026, 2);
  assert.deepEqual(grid[0], [null, null, null, null, null, null, '2026-02-01']);
  assert.deepEqual(grid.at(-1), ['2026-02-23', '2026-02-24', '2026-02-25', '2026-02-26', '2026-02-27', '2026-02-28', null]);
  assert.equal(monthGrid(2024, 2).flat().filter(Boolean).length, 29);
  assert.equal(monthGrid(2021, 2).length, 4);
});

test('month navigation crosses years in both directions', () => {
  assert.deepEqual(monthOf('2026-01-31'), { year: 2026, month: 1 });
  assert.deepEqual(shiftMonth(2026, 1, -1), { year: 2025, month: 12 });
  assert.deepEqual(shiftMonth(2026, 12, 14), { year: 2028, month: 2 });
});

test('day states follow the requested precedence', () => {
  const day = { date: '2026-02-01', rows: 1, unjudged: 1 };
  assert.equal(dayState(day, day.date), 'today');
  assert.equal(dayState({ ...day, dreamed: true, scanned: true }, '2026-02-02'), 'dreamed');
  assert.equal(dayState({ ...day, dreamed: false, state: 'dreamed' }, '2026-02-02'), 'dreamed');
  assert.equal(dayState({ ...day, scanned: true }, '2026-02-02'), 'scanned');
  assert.equal(dayState(day, '2026-02-02'), 'waiting');
  assert.equal(dayState({ ...day, rows: 0 }, '2026-02-02'), 'waiting');
  assert.equal(dayState({ ...day, rows: 0, unjudged: 0 }, '2026-02-02'), 'empty');
  assert.equal(dayState(undefined, '2026-02-02'), 'empty');
});
