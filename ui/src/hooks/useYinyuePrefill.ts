import { useEffect, useMemo, useRef, useState } from 'react';
import { floorOf, prefillAfter, type TableMessage } from '../lib/chatMentions.mts';

/** How often a pre-fill that has gone quiet is noticed (the phone's 30 s). */
const RECHECK_MS = 30_000;

/**
 * The composer's `@Yinyue` pre-fill (the phone's chat_screen `_syncPrefill`):
 * when who holds the table changes, or the quiet window ends, the box takes
 * the wanted mention — but only over an empty box or the mention it wrote
 * itself, never over the person's own words, and a mention they deleted stays
 * deleted until the floor changes again.
 *
 * `label` is how her name is written after `@`; empty when she is not at this
 * table. Returns `spent()` — call it when the box is emptied by a send, so the
 * next floor state refills it.
 */
export function useYinyuePrefill(
  messages: TableMessage[],
  tableIds: string[],
  label: string,
  input: string,
  setInput: (value: string) => void,
): () => void {
  const [now, setNow] = useState(() => Date.now());
  const written = useRef('');
  const tableKey = tableIds.join(',');
  const floor = useMemo(() => floorOf(messages, tableIds), [messages, tableKey]); // eslint-disable-line react-hooks/exhaustive-deps
  const want = label ? prefillAfter(floor, now, label) : '';

  useEffect(() => {
    const timer = window.setInterval(() => setNow(Date.now()), RECHECK_MS);
    return () => window.clearInterval(timer);
  }, []);

  useEffect(() => {
    if (want === written.current) return;
    if (input === '' || input === written.current) setInput(want);
    written.current = want;
  }, [want]); // eslint-disable-line react-hooks/exhaustive-deps

  return () => { written.current = ''; };
}
