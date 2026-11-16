import type { MemoryDay } from './memoryApi.mts';

/** Months use the human convention: January is 1. */
export function monthOf(isoDate: string): { year: number; month: number } {
  return { year: Number(isoDate.slice(0, 4)), month: Number(isoDate.slice(5, 7)) };
}

export function shiftMonth(year: number, month: number, delta: number): { year: number; month: number } {
  const index = year * 12 + month - 1 + delta;
  return { year: Math.floor(index / 12), month: ((index % 12) + 12) % 12 + 1 };
}

export function monthGrid(year: number, month: number): (string | null)[][] {
  const first = new Date(Date.UTC(year, month - 1, 1));
  const offset = (first.getUTCDay() + 6) % 7;
  const count = new Date(Date.UTC(year, month, 0)).getUTCDate();
  const cells: (string | null)[] = Array(offset).fill(null);
  for (let day = 1; day <= count; day++) {
    cells.push(`${String(year).padStart(4, '0')}-${String(month).padStart(2, '0')}-${String(day).padStart(2, '0')}`);
  }
  while (cells.length % 7) cells.push(null);
  return Array.from({ length: cells.length / 7 }, (_, i) => cells.slice(i * 7, i * 7 + 7));
}

export type DayState = 'today' | 'dreamed' | 'scanned' | 'waiting' | 'empty';
export function dayState(day: Pick<MemoryDay, 'date' | 'dreamed' | 'state' | 'scanned' | 'rows' | 'unjudged'> | undefined, today: string): DayState {
  if (day?.date === today) return 'today';
  if (day?.dreamed || day?.state === 'dreamed') return 'dreamed';
  if (day?.scanned) return 'scanned';
  if (day && (day.rows > 0 || day.unjudged > 0)) return 'waiting';
  return 'empty';
}
