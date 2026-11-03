import { useEffect, useMemo, useRef, useState } from 'react';
import { floorOf, prefillAfter, type TableMessage } from '../lib/chatMentions.mts';

/** How often a pre-fill that has gone quiet is noticed (the phone's 30 s). */
const RECHECK_MS = 30_000;

/**
 * The composer's Yinyue pre-fill (the phone's chat_screen `_syncPrefill`):
 * when who holds the table changes, or the quiet window ends, the box's
 * addressee chip takes the wanted agent — but only over an empty box whose
 * chip is empty or the one it set itself, never over the person's own pick,
 * and a chip they cleared stays cleared until the floor changes again.
 *
 * `here` says she is at this table. Returns `spent()` — call it when the box
 * is emptied by a send, so the next floor state refills it.
 */
export function useYinyuePrefill(
  messages: TableMessage[],
  tableIds: string[],
  here: boolean,
  input: string,
  chip: string,
  setChip: (agent: string) => void,
): () => void {
  const [now, setNow] = useState(() => Date.now());
  const written = useRef('');
  // The caller builds `tableIds` fresh each render; key the memo on its
  // contents, and read the ids back from that key so the deps are exact.
  const tableKey = tableIds.join(',');
  const floor = useMemo(() => floorOf(messages, tableKey.split(',')), [messages, tableKey]);
  const want = here ? prefillAfter(floor, now) : '';

  useEffect(() => {
    const timer = window.setInterval(() => setNow(Date.now()), RECHECK_MS);
    return () => window.clearInterval(timer);
  }, []);

  // Also on each new exchange (`floor.at`): after a send spent the chip,
  // her next line refills it though `want` stayed the same.
  useEffect(() => {
    if (want === written.current) return;
    if (input === '' && (chip === '' || chip === written.current)) setChip(want);
    written.current = want;
  }, [want, floor.at]); // eslint-disable-line react-hooks/exhaustive-deps

  return () => { written.current = ''; };
}
